//! A single queue: its settings, its messages and the rules that act on
//! them without reaching into other queues.

use std::borrow::Borrow;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use crate::ids::{Ids, MessageId, ReceiptHandle};
use crate::request::Attributes;
use crate::response::{AttributesView, Counts, ReceivedMessage};

/// Most messages one queue holds, in any state.
pub(crate) const MAX_MESSAGES: usize = 100_000;

/// A send is a duplicate while less than this many seconds have passed
/// since the original was accepted.
const DEDUPLICATION_WINDOW: u64 = 300;

const DEFAULT_VISIBILITY_TIMEOUT: u32 = 30;
const DEFAULT_RETENTION_SECONDS: u32 = 345_600;

/// A queue name that follows the naming rules.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct QueueName(String);

impl QueueName {
    const FIFO_SUFFIX: &'static str = ".fifo";

    pub(crate) fn parse(text: &str) -> Option<Self> {
        let is_plain = |s: &str| {
            s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        };
        let valid = match text.strip_suffix(Self::FIFO_SUFFIX) {
            Some(stem) => (1..=75).contains(&stem.len()) && is_plain(stem),
            None => (1..=80).contains(&text.len()) && is_plain(text),
        };
        valid.then(|| Self(text.to_owned()))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

    pub(crate) fn has_fifo_suffix(&self) -> bool {
        self.0.ends_with(Self::FIFO_SUFFIX)
    }
}

// Lets the queue map be searched with the raw `&str` of a request. `Hash`,
// `Eq` and `Ord` are derived from the inner `String`, so they agree with `str`.
impl Borrow<str> for QueueName {
    fn borrow(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone)]
pub(crate) struct DeadLetter {
    pub queue: QueueName,
    pub max_receive_count: u32,
}

/// The attributes of a queue.
#[derive(Debug)]
pub(crate) struct Settings {
    pub visibility_timeout: u32,
    pub delay_seconds: u32,
    pub retention_seconds: u32,
    pub fifo: bool,
    pub content_based_deduplication: bool,
    pub dead_letter: Option<DeadLetter>,
}

impl Settings {
    /// Settings for a new queue: attributes left out take their defaults.
    pub(crate) fn new(attributes: &Attributes, dead_letter: Option<DeadLetter>) -> Self {
        Self {
            visibility_timeout: attributes
                .visibility_timeout
                .unwrap_or(DEFAULT_VISIBILITY_TIMEOUT),
            delay_seconds: attributes.delay_seconds.unwrap_or(0),
            retention_seconds: attributes
                .retention_seconds
                .unwrap_or(DEFAULT_RETENTION_SECONDS),
            fifo: attributes.fifo.unwrap_or(false),
            content_based_deduplication: attributes.content_based_deduplication.unwrap_or(false),
            dead_letter,
        }
    }

    /// Whether `Settings::new(attributes, ..)` would equal these settings.
    pub(crate) fn matches(&self, attributes: &Attributes) -> bool {
        let dead_letter = self
            .dead_letter
            .as_ref()
            .map(|d| (d.queue.as_str(), d.max_receive_count));
        let requested = attributes
            .dead_letter_queue
            .as_deref()
            .zip(attributes.max_receive_count);
        self.visibility_timeout
            == attributes
                .visibility_timeout
                .unwrap_or(DEFAULT_VISIBILITY_TIMEOUT)
            && self.delay_seconds == attributes.delay_seconds.unwrap_or(0)
            && self.retention_seconds
                == attributes
                    .retention_seconds
                    .unwrap_or(DEFAULT_RETENTION_SECONDS)
            && self.fifo == attributes.fifo.unwrap_or(false)
            && self.content_based_deduplication
                == attributes.content_based_deduplication.unwrap_or(false)
            && dead_letter == requested
    }

    /// Overwrites the attributes present in `attributes`. `Fifo` is never
    /// changed; `dead_letter` replaces the configuration when it is `Some`.
    pub(crate) fn update(&mut self, attributes: &Attributes, dead_letter: Option<DeadLetter>) {
        if let Some(v) = attributes.visibility_timeout {
            self.visibility_timeout = v;
        }
        if let Some(v) = attributes.delay_seconds {
            self.delay_seconds = v;
        }
        if let Some(v) = attributes.retention_seconds {
            self.retention_seconds = v;
        }
        if let Some(v) = attributes.content_based_deduplication {
            self.content_based_deduplication = v;
        }
        if dead_letter.is_some() {
            self.dead_letter = dead_letter;
        }
    }

    pub(crate) fn view(&self) -> AttributesView {
        AttributesView {
            visibility_timeout: self.visibility_timeout,
            delay_seconds: self.delay_seconds,
            retention_seconds: self.retention_seconds,
            fifo: self.fifo,
            content_based_deduplication: self.content_based_deduplication,
            dead_letter_queue: self
                .dead_letter
                .as_ref()
                .map(|d| d.queue.as_str().to_owned()),
            max_receive_count: self.dead_letter.as_ref().map(|d| d.max_receive_count),
        }
    }
}

#[derive(Debug)]
struct Delivery {
    handle: ReceiptHandle,
    /// The message is in flight while `now` is below this.
    until: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Delayed,
    InFlight,
    Visible,
}

#[derive(Debug)]
pub(crate) struct Message {
    id: MessageId,
    body: String,
    sent_at: u64,
    receive_count: u32,
    first_received_at: Option<u64>,
    group: Option<String>,
    sequence: Option<u64>,
    /// The message is delayed while `now` is below this.
    visible_from: u64,
    delivery: Option<Delivery>,
}

impl Message {
    pub(crate) fn new(
        id: MessageId,
        body: String,
        group: Option<String>,
        now: u64,
        delay_seconds: u32,
    ) -> Self {
        Self {
            id,
            body,
            sent_at: now,
            receive_count: 0,
            first_received_at: None,
            group,
            sequence: None,
            visible_from: now + u64::from(delay_seconds),
            delivery: None,
        }
    }

    fn state(&self, now: u64) -> State {
        if now < self.visible_from {
            State::Delayed
        } else if self.delivery.as_ref().is_some_and(|d| now < d.until) {
            State::InFlight
        } else {
            State::Visible
        }
    }
}

/// What an earlier send with the same deduplication id was given.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Accepted {
    pub message_id: MessageId,
    pub sequence: Option<u64>,
    at: u64,
}

/// The messages `Queue::receive` took out of the queue.
pub(crate) struct Received {
    pub delivered: Vec<ReceivedMessage>,
    /// Messages that exceeded the receive count; the caller hands them to
    /// the dead-letter queue.
    pub dead: Vec<Message>,
}

#[derive(Debug)]
pub(crate) struct Queue {
    pub settings: Settings,
    /// Messages by arrival key, which grows with every message added.
    messages: BTreeMap<u64, Message>,
    /// `(SentAt, arrival key)` of every message, to find expired ones fast.
    by_sent_at: BTreeSet<(u64, u64)>,
    /// Latest handle of every delivered message, mapped to its arrival key.
    handles: HashMap<ReceiptHandle, u64>,
    /// Deduplication ids outlive their messages.
    accepted: HashMap<String, Accepted>,
    next_key: u64,
    next_sequence: u64,
}

impl Queue {
    pub(crate) fn new(settings: Settings) -> Self {
        Self {
            settings,
            messages: BTreeMap::new(),
            by_sent_at: BTreeSet::new(),
            handles: HashMap::new(),
            accepted: HashMap::new(),
            next_key: 0,
            next_sequence: 1,
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.messages.len()
    }

    /// Removes the messages whose retention has run out. Called before every
    /// use of the queue, which is enough because retention only changes
    /// through a command that uses the queue.
    pub(crate) fn expire(&mut self, now: u64) {
        let retention = u64::from(self.settings.retention_seconds);
        while let Some(&(sent_at, key)) = self.by_sent_at.first() {
            if now < sent_at + retention {
                break;
            }
            self.remove(key);
        }
    }

    pub(crate) fn counts(&self, now: u64) -> Counts {
        let mut counts = Counts::default();
        for message in self.messages.values() {
            match message.state(now) {
                State::Visible => counts.visible += 1,
                State::InFlight => counts.in_flight += 1,
                State::Delayed => counts.delayed += 1,
            }
        }
        counts
    }

    pub(crate) fn purge(&mut self) {
        self.messages.clear();
        self.by_sent_at.clear();
        self.handles.clear();
    }

    /// Adds a message at the end and returns its sequence number on a FIFO
    /// queue.
    pub(crate) fn push(&mut self, mut message: Message) -> Option<u64> {
        message.sequence = self.settings.fifo.then(|| {
            let sequence = self.next_sequence;
            self.next_sequence += 1;
            sequence
        });
        let sequence = message.sequence;
        self.by_sent_at.insert((message.sent_at, self.next_key));
        self.messages.insert(self.next_key, message);
        self.next_key += 1;
        sequence
    }

    /// Adds a message moved from another queue: it starts over as a fresh,
    /// immediately visible message that keeps its id, body, time and group.
    pub(crate) fn push_dead_letter(&mut self, mut message: Message) {
        message.receive_count = 0;
        message.first_received_at = None;
        message.visible_from = 0;
        message.delivery = None;
        self.push(message);
    }

    fn remove(&mut self, key: u64) -> Option<Message> {
        let message = self.messages.remove(&key)?;
        self.by_sent_at.remove(&(message.sent_at, key));
        if let Some(delivery) = &message.delivery {
            self.handles.remove(&delivery.handle);
        }
        Some(message)
    }

    /// The send accepted under `deduplication_id` less than the window ago.
    pub(crate) fn duplicate_of(&self, deduplication_id: &str, now: u64) -> Option<Accepted> {
        self.accepted
            .get(deduplication_id)
            .filter(|accepted| now - accepted.at < DEDUPLICATION_WINDOW)
            .copied()
    }

    pub(crate) fn record_accepted(
        &mut self,
        deduplication_id: String,
        message_id: MessageId,
        sequence: Option<u64>,
        now: u64,
    ) {
        let accepted = Accepted {
            message_id,
            sequence,
            at: now,
        };
        self.accepted.insert(deduplication_id, accepted);
    }

    /// Delivers up to `max` eligible messages in arrival order.
    pub(crate) fn receive(
        &mut self,
        now: u64,
        max: usize,
        visibility_timeout: u32,
        ids: &mut Ids,
    ) -> Received {
        let max_receive_count = self
            .settings
            .dead_letter
            .as_ref()
            .map(|d| d.max_receive_count);
        let in_flight_until = now + u64::from(visibility_timeout);
        let mut delivered = Vec::new();
        let mut dead_keys = Vec::new();
        // Groups that still have an earlier message in the queue.
        let mut occupied_groups: HashSet<String> = HashSet::new();

        for (&key, message) in &mut self.messages {
            if delivered.len() == max {
                break;
            }
            let group_free = message
                .group
                .as_ref()
                .is_none_or(|group| !occupied_groups.contains(group));
            if group_free && message.state(now) == State::Visible {
                if max_receive_count.is_some_and(|limit| message.receive_count >= limit) {
                    dead_keys.push(key);
                    continue;
                }
                let handle = ids.next_receipt();
                if let Some(previous) = message.delivery.replace(Delivery {
                    handle,
                    until: in_flight_until,
                }) {
                    self.handles.remove(&previous.handle);
                }
                self.handles.insert(handle, key);
                message.receive_count += 1;
                let first_received_at = *message.first_received_at.get_or_insert(now);
                delivered.push(ReceivedMessage {
                    message_id: message.id,
                    receipt_handle: handle,
                    body: message.body.clone(),
                    receive_count: message.receive_count,
                    sent_at: message.sent_at,
                    first_received_at,
                    group_id: message.group.clone(),
                    sequence_number: message.sequence,
                });
            }
            if let Some(group) = &message.group
                && !occupied_groups.contains(group)
            {
                occupied_groups.insert(group.clone());
            }
        }

        let dead = dead_keys
            .into_iter()
            .map(|key| {
                self.remove(key)
                    .expect("dead keys were collected from this queue just now")
            })
            .collect();
        Received { delivered, dead }
    }

    /// The arrival key of the message that `handle` was issued for, if that
    /// is its latest handle and the message is in flight.
    pub(crate) fn in_flight(&self, handle: ReceiptHandle, now: u64) -> Option<u64> {
        let key = *self.handles.get(&handle)?;
        let message = self.messages.get(&key)?;
        (message.state(now) == State::InFlight).then_some(key)
    }

    pub(crate) fn delete(&mut self, key: u64) {
        self.remove(key);
    }

    pub(crate) fn change_visibility(&mut self, key: u64, now: u64, visibility_timeout: u32) {
        if let Some(delivery) = self
            .messages
            .get_mut(&key)
            .and_then(|message| message.delivery.as_mut())
        {
            delivery.until = now + u64::from(visibility_timeout);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_name_syntax() {
        let long_plain = "a".repeat(80);
        let long_fifo = format!("{}.fifo", "a".repeat(75));
        let valid = ["a", "A_z-9", &long_plain, "q.fifo", &long_fifo];
        for name in valid {
            assert!(QueueName::parse(name).is_some(), "{name}");
        }
        let too_long_plain = "a".repeat(81);
        let too_long_fifo = format!("{}.fifo", "a".repeat(76));
        let invalid = [
            "",
            ".fifo",
            "a.b",
            "a b",
            "q.fifo.fifo",
            "q.FIFO",
            "é",
            &too_long_plain,
            &too_long_fifo,
        ];
        for name in invalid {
            assert!(QueueName::parse(name).is_none(), "{name}");
        }
    }
}
