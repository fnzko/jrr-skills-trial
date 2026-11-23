//! The queue model: settings, messages, retention and the receive scan.
//!
//! Nothing here knows about JSON or about other queues; moving a message to
//! a dead-letter queue is orchestrated by the broker.

use std::borrow::Borrow;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use crate::types::{MessageId, ReceiptHandle, Seconds, SequenceNumber};

pub const MAX_MESSAGES_PER_QUEUE: usize = 100_000;
pub const DEDUP_WINDOW: Seconds = Seconds::new(300);
const FIFO_SUFFIX: &str = ".fifo";
const MAX_NAME_LEN: usize = 80;

/// A syntactically valid queue name (section 4 of the spec).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct QueueName(String);

impl QueueName {
    pub fn parse(text: &str) -> Option<Self> {
        let stem = text.strip_suffix(FIFO_SUFFIX).unwrap_or(text);
        let stem_ok = !stem.is_empty() && stem.bytes().all(is_name_byte);
        (stem_ok && text.len() <= MAX_NAME_LEN).then(|| Self(text.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn has_fifo_suffix(&self) -> bool {
        self.0.ends_with(FIFO_SUFFIX)
    }
}

/// Lets the queue table be searched with any string, valid name or not.
impl Borrow<str> for QueueName {
    fn borrow(&self) -> &str {
        &self.0
    }
}

pub fn is_name_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-'
}

/// Internal identity of a queue; a queue recreated under an old name gets a new one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueueUid(u64);

impl QueueUid {
    pub fn new(n: u64) -> Self {
        Self(n)
    }
}

/// Position of a message in its queue's arrival order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Arrival(u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueType {
    Standard,
    Fifo { content_based_dedup: bool },
}

impl QueueType {
    pub fn is_fifo(self) -> bool {
        matches!(self, Self::Fifo { .. })
    }

    pub fn content_based_dedup(self) -> bool {
        matches!(
            self,
            Self::Fifo {
                content_based_dedup: true
            }
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeadLetter {
    pub queue: QueueName,
    pub max_receive_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub visibility_timeout: Seconds,
    pub delay: Seconds,
    pub retention: Seconds,
    pub queue_type: QueueType,
    pub dead_letter: Option<DeadLetter>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FifoInfo {
    pub group: String,
    pub sequence: SequenceNumber,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageState {
    Delayed,
    InFlight,
    Visible,
}

#[derive(Debug, Clone)]
pub struct Message {
    pub id: MessageId,
    pub body: String,
    pub sent_at: Seconds,
    pub receive_count: u32,
    pub first_received_at: Option<Seconds>,
    pub fifo: Option<FifoInfo>,
    /// Not receivable before this instant.
    available_at: Seconds,
    /// In flight before this instant.
    invisible_until: Seconds,
    receipt: Option<ReceiptHandle>,
}

impl Message {
    pub fn new(
        id: MessageId,
        body: String,
        sent_at: Seconds,
        delay: Seconds,
        fifo: Option<FifoInfo>,
    ) -> Self {
        Self {
            id,
            body,
            sent_at,
            receive_count: 0,
            first_received_at: None,
            fifo,
            available_at: sent_at.plus(delay),
            invisible_until: Seconds::ZERO,
            receipt: None,
        }
    }

    pub fn state(&self, now: Seconds) -> MessageState {
        if now < self.available_at {
            MessageState::Delayed
        } else if now < self.invisible_until {
            MessageState::InFlight
        } else {
            MessageState::Visible
        }
    }

    /// True when `handle` is this message's latest handle and its visibility
    /// timeout has not ended.
    pub fn is_in_flight_as(&self, handle: ReceiptHandle, now: Seconds) -> bool {
        self.receipt == Some(handle) && self.state(now) == MessageState::InFlight
    }

    /// Ends the visibility timeout `timeout` from `now`.
    pub fn set_visibility(&mut self, now: Seconds, timeout: Seconds) {
        self.invisible_until = now.plus(timeout);
    }

    /// The message as it enters a dead-letter queue.
    fn reset_for_dead_letter(&mut self, fifo: Option<FifoInfo>) {
        self.receive_count = 0;
        self.first_received_at = None;
        self.fifo = fifo;
        self.available_at = Seconds::ZERO;
        self.invisible_until = Seconds::ZERO;
        self.receipt = None;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Counts {
    pub visible: usize,
    pub in_flight: usize,
    pub delayed: usize,
}

#[derive(Debug, Clone, Copy)]
struct DedupEntry {
    accepted_at: Seconds,
    message_id: MessageId,
    sequence: SequenceNumber,
}

/// Result of the receive scan: which messages were delivered, and which were
/// taken out of the queue for the dead-letter queue.
#[derive(Debug)]
pub struct Scan {
    pub delivered: Vec<Arrival>,
    pub dead: Vec<Message>,
}

#[derive(Debug)]
pub struct Queue {
    uid: QueueUid,
    settings: Settings,
    messages: BTreeMap<Arrival, Message>,
    next_arrival: Arrival,
    /// `(sent_at, arrival)` of every message held, oldest first, for retention.
    by_age: BTreeSet<(Seconds, Arrival)>,
    next_sequence: SequenceNumber,
    dedup: HashMap<String, DedupEntry>,
}

enum Action {
    Deliver,
    DeadLetter,
}

impl Queue {
    pub fn new(uid: QueueUid, settings: Settings) -> Self {
        Self {
            uid,
            settings,
            messages: BTreeMap::new(),
            next_arrival: Arrival(0),
            by_age: BTreeSet::new(),
            next_sequence: SequenceNumber::FIRST,
            dedup: HashMap::new(),
        }
    }

    pub fn uid(&self) -> QueueUid {
        self.uid
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    /// Replaces the settings. The caller expires messages again afterwards,
    /// because a lower retention applies to messages already held.
    pub fn set_settings(&mut self, settings: Settings) {
        self.settings = settings;
    }

    pub fn len(&self) -> usize {
        self.messages.len()
    }

    pub fn message(&self, arrival: Arrival) -> Option<&Message> {
        self.messages.get(&arrival)
    }

    pub fn message_mut(&mut self, arrival: Arrival) -> Option<&mut Message> {
        self.messages.get_mut(&arrival)
    }

    pub fn counts(&self, now: Seconds) -> Counts {
        self.messages
            .values()
            .fold(Counts::default(), |mut counts, m| {
                match m.state(now) {
                    MessageState::Visible => counts.visible += 1,
                    MessageState::InFlight => counts.in_flight += 1,
                    MessageState::Delayed => counts.delayed += 1,
                }
                counts
            })
    }

    /// Removes every message that has outlived the retention period.
    pub fn expire(&mut self, now: Seconds) {
        while let Some(&(sent_at, arrival)) = self.by_age.first() {
            if now < sent_at.plus(self.settings.retention) {
                break;
            }
            self.by_age.pop_first();
            self.messages.remove(&arrival);
        }
    }

    pub fn purge(&mut self) {
        self.messages.clear();
        self.by_age.clear();
    }

    pub fn remove(&mut self, arrival: Arrival) -> Option<Message> {
        let message = self.messages.remove(&arrival)?;
        self.by_age.remove(&(message.sent_at, arrival));
        Some(message)
    }

    /// Adds a message at the end of the queue.
    pub fn push(&mut self, message: Message) -> Arrival {
        let arrival = self.next_arrival;
        self.next_arrival = Arrival(arrival.0 + 1);
        self.by_age.insert((message.sent_at, arrival));
        self.messages.insert(arrival, message);
        arrival
    }

    /// The next sequence number of a FIFO queue.
    pub fn take_sequence(&mut self) -> SequenceNumber {
        let sequence = self.next_sequence;
        self.next_sequence = sequence.next();
        sequence
    }

    /// The message accepted under `dedup_id` less than the dedup window ago.
    pub fn recent_duplicate(
        &self,
        dedup_id: &str,
        now: Seconds,
    ) -> Option<(MessageId, SequenceNumber)> {
        let entry = self.dedup.get(dedup_id)?;
        (now.minus(entry.accepted_at) < DEDUP_WINDOW).then_some((entry.message_id, entry.sequence))
    }

    /// Opens a new dedup window for `dedup_id`, replacing an elapsed one.
    pub fn record_dedup(
        &mut self,
        dedup_id: String,
        now: Seconds,
        message_id: MessageId,
        sequence: SequenceNumber,
    ) {
        self.dedup.insert(
            dedup_id,
            DedupEntry {
                accepted_at: now,
                message_id,
                sequence,
            },
        );
    }

    /// Adds a message arriving from another queue's dead-letter move.
    pub fn push_dead_letter(&mut self, mut message: Message, now: Seconds) {
        let fifo = message.fifo.take().map(|old| FifoInfo {
            group: old.group,
            sequence: self.take_sequence(),
        });
        message.reset_for_dead_letter(fifo);
        self.push(message);
        self.expire(now);
    }

    /// Examines the messages once each in arrival order and delivers up to
    /// `max` of them. Handles are assigned by the caller afterwards.
    pub fn scan(&mut self, now: Seconds, max: usize, visibility: Seconds) -> Scan {
        let actions = self.plan_scan(now, max);
        let mut scan = Scan {
            delivered: Vec::new(),
            dead: Vec::new(),
        };
        for (arrival, action) in actions {
            match action {
                Action::Deliver => {
                    if let Some(m) = self.messages.get_mut(&arrival) {
                        m.receive_count += 1;
                        m.first_received_at.get_or_insert(now);
                        m.set_visibility(now, visibility);
                        scan.delivered.push(arrival);
                    }
                }
                Action::DeadLetter => scan.dead.extend(self.remove(arrival)),
            }
        }
        scan
    }

    /// Decides every message's fate before any is changed: the decisions of
    /// one scan depend only on the state at its start.
    fn plan_scan(&self, now: Seconds, max: usize) -> Vec<(Arrival, Action)> {
        let max_receive = self
            .settings
            .dead_letter
            .as_ref()
            .map(|d| d.max_receive_count);
        let mut blocked_groups: HashSet<&str> = HashSet::new();
        let mut actions = Vec::new();
        let mut deliveries = 0;
        for (&arrival, message) in &self.messages {
            if deliveries == max {
                break;
            }
            let group = message.fifo.as_ref().map(|f| f.group.as_str());
            let eligible = message.state(now) == MessageState::Visible
                && group.is_none_or(|g| !blocked_groups.contains(g));
            if eligible {
                if max_receive.is_some_and(|limit| message.receive_count >= limit) {
                    // It leaves the queue, so it does not hold back its group.
                    actions.push((arrival, Action::DeadLetter));
                    continue;
                }
                actions.push((arrival, Action::Deliver));
                deliveries += 1;
            }
            blocked_groups.extend(group);
        }
        actions
    }

    /// Records the handle issued for a delivered message.
    pub fn set_receipt(&mut self, arrival: Arrival, handle: ReceiptHandle) {
        if let Some(m) = self.messages.get_mut(&arrival) {
            m.receipt = Some(handle);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_name_syntax() {
        let long_ok = "a".repeat(80);
        let long_bad = "a".repeat(81);
        let fifo_ok = format!("{}.fifo", "a".repeat(75));
        let fifo_bad = format!("{}.fifo", "a".repeat(76));
        let cases = [
            ("jobs", true),
            ("A_b-9", true),
            ("jobs.fifo", true),
            (long_ok.as_str(), true),
            (long_bad.as_str(), false),
            (fifo_ok.as_str(), true),
            (fifo_bad.as_str(), false),
            ("", false),
            (".fifo", false),
            ("a.b", false),
            ("a.fifo.fifo", false),
            ("a b", false),
            ("é", false),
        ];
        for (name, valid) in cases {
            assert_eq!(QueueName::parse(name).is_some(), valid, "{name:?}");
        }
    }
}
