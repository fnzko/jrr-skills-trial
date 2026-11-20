//! Queue state: settings, messages in arrival order, retention and deduplication.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::rc::Rc;

use serde_json::{Map, Value};

use crate::error::{Error, ErrorCode, Result};
use crate::params::{Params, invalid};

/// Most messages one queue may hold, in any state.
pub const MAX_MESSAGES: usize = 100_000;
/// Seconds during which a FIFO deduplication id suppresses repeats.
const DEDUP_WINDOW: u64 = 300;

const NAME_CHARS: fn(char) -> bool = |c| c.is_ascii_alphanumeric() || c == '_' || c == '-';
const FIFO_SUFFIX: &str = ".fifo";

/// Whether `name` follows the queue naming rules.
pub fn is_valid_queue_name(name: &str) -> bool {
    let stem = name.strip_suffix(FIFO_SUFFIX).unwrap_or(name);
    let limit = if stem.len() == name.len() { 80 } else { 75 };
    (1..=limit).contains(&stem.len()) && stem.chars().all(NAME_CHARS)
}

pub fn has_fifo_suffix(name: &str) -> bool {
    name.ends_with(FIFO_SUFFIX)
}

/// Whether `text` is usable as a `GroupId` or `DeduplicationId`.
pub fn is_valid_identifier(text: &str) -> bool {
    (1..=128).contains(&text.len()) && text.chars().all(NAME_CHARS)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeadLetter {
    pub queue: String,
    pub max_receive_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub visibility_timeout: u32,
    pub delay_seconds: u32,
    pub retention_seconds: u32,
    pub fifo: bool,
    pub content_based_deduplication: bool,
    pub dead_letter: Option<DeadLetter>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            visibility_timeout: 30,
            delay_seconds: 0,
            retention_seconds: 345_600,
            fifo: false,
            content_based_deduplication: false,
            dead_letter: None,
        }
    }
}

/// The attributes a request actually named, each already checked on its own.
#[derive(Debug, Default)]
pub struct AttributePatch {
    pub visibility_timeout: Option<u32>,
    pub delay_seconds: Option<u32>,
    pub retention_seconds: Option<u32>,
    pub fifo: Option<bool>,
    pub content_based_deduplication: Option<bool>,
    pub dead_letter_queue: Option<String>,
    pub max_receive_count: Option<u32>,
}

const ATTRIBUTE_NAMES: [&str; 7] = [
    "VisibilityTimeout",
    "DelaySeconds",
    "RetentionSeconds",
    "Fifo",
    "ContentBasedDeduplication",
    "DeadLetterQueue",
    "MaxReceiveCount",
];

impl AttributePatch {
    pub fn parse(attributes: &Map<String, Value>) -> Result<Self> {
        if let Some(unknown) = attributes
            .keys()
            .find(|key| !ATTRIBUTE_NAMES.contains(&key.as_str()))
        {
            return Err(invalid(format!("unknown attribute {unknown:?}")));
        }
        let p = Params::new(attributes);
        let number = |name, range| p.integer(name, range).map(|n| n.map(|n| n as u32));
        Ok(Self {
            visibility_timeout: number("VisibilityTimeout", 0..=43_200)?,
            delay_seconds: number("DelaySeconds", 0..=900)?,
            retention_seconds: number("RetentionSeconds", 60..=1_209_600)?,
            fifo: p.boolean("Fifo")?,
            content_based_deduplication: p.boolean("ContentBasedDeduplication")?,
            dead_letter_queue: p.string("DeadLetterQueue")?.map(str::to_owned),
            max_receive_count: number("MaxReceiveCount", 1..=1000)?,
        })
    }

    /// The requested dead-letter configuration; naming only half of it is an error.
    pub fn dead_letter(&self) -> Result<Option<DeadLetter>> {
        match (&self.dead_letter_queue, self.max_receive_count) {
            (Some(queue), Some(max_receive_count)) => Ok(Some(DeadLetter {
                queue: queue.clone(),
                max_receive_count,
            })),
            (None, None) => Ok(None),
            _ => Err(invalid(
                "DeadLetterQueue and MaxReceiveCount must be given together".into(),
            )),
        }
    }

    /// Applies the patch over `base`; `dead_letter` comes from [`Self::dead_letter`].
    pub fn apply(&self, mut base: Settings, dead_letter: Option<DeadLetter>) -> Settings {
        base.visibility_timeout = self.visibility_timeout.unwrap_or(base.visibility_timeout);
        base.delay_seconds = self.delay_seconds.unwrap_or(base.delay_seconds);
        base.retention_seconds = self.retention_seconds.unwrap_or(base.retention_seconds);
        base.fifo = self.fifo.unwrap_or(base.fifo);
        base.content_based_deduplication = self
            .content_based_deduplication
            .unwrap_or(base.content_based_deduplication);
        if dead_letter.is_some() {
            base.dead_letter = dead_letter;
        }
        base
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Delayed,
    InFlight,
    Visible,
}

#[derive(Debug, Clone)]
pub struct Message {
    /// Number `n` of the identifier `m-<n>`.
    pub id: u64,
    pub body: String,
    pub sent_at: u64,
    pub receive_count: u32,
    pub first_received_at: Option<u64>,
    pub group_id: Option<String>,
    pub sequence_number: Option<u64>,
    delayed_until: u64,
    pub in_flight_until: u64,
    /// Number `n` of the latest handle `r-<n>`, if the message was ever delivered.
    pub latest_handle: Option<u64>,
}

impl Message {
    pub fn new(id: u64, body: String, now: u64, delay: u32, group_id: Option<String>) -> Self {
        Self {
            id,
            body,
            sent_at: now,
            receive_count: 0,
            first_received_at: None,
            group_id,
            sequence_number: None,
            delayed_until: now + u64::from(delay),
            in_flight_until: 0,
            latest_handle: None,
        }
    }

    pub fn state(&self, now: u64) -> State {
        if now < self.delayed_until {
            State::Delayed
        } else if now < self.in_flight_until {
            State::InFlight
        } else {
            State::Visible
        }
    }

    /// The message as it arrives in a dead-letter queue: fresh, visible, never received.
    fn reset_for_dead_letter(&mut self) {
        self.receive_count = 0;
        self.first_received_at = None;
        self.delayed_until = 0;
        self.in_flight_until = 0;
        self.latest_handle = None;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Counts {
    pub visible: usize,
    pub in_flight: usize,
    pub delayed: usize,
}

#[derive(Debug)]
struct DedupEntry {
    at: u64,
    message_id: u64,
    sequence_number: u64,
}

/// FIFO deduplication ids of one queue; independent of the messages themselves.
#[derive(Debug, Default)]
struct Dedup {
    entries: HashMap<Rc<str>, DedupEntry>,
    /// Records in time order, so expired ones can be dropped from the front.
    order: VecDeque<(u64, Rc<str>)>,
}

impl Dedup {
    fn find(&self, id: &str, now: u64) -> Option<&DedupEntry> {
        self.entries
            .get(id)
            .filter(|entry| now - entry.at < DEDUP_WINDOW)
    }

    fn record(&mut self, id: &str, entry: DedupEntry, now: u64) {
        while let Some((at, old)) = self.order.front() {
            if now - at < DEDUP_WINDOW {
                break;
            }
            if self.entries.get(old).is_some_and(|e| e.at == *at) {
                self.entries.remove(old);
            }
            self.order.pop_front();
        }
        let id: Rc<str> = Rc::from(id);
        self.order.push_back((entry.at, Rc::clone(&id)));
        self.entries.insert(id, entry);
    }
}

/// The original message of a duplicate send.
#[derive(Debug, Clone, Copy)]
pub struct Original {
    pub message_id: u64,
    pub sequence_number: u64,
}

/// What one `ReceiveMessage` scan did to a queue.
#[derive(Debug, Default)]
pub struct Received {
    /// Arrival keys and snapshots of the delivered messages, in delivery order.
    pub delivered: Vec<(u64, Message)>,
    /// Messages removed from the queue for exceeding `MaxReceiveCount`.
    pub dead: Vec<Message>,
}

#[derive(Debug)]
pub struct Queue {
    /// Distinguishes this queue from an earlier one of the same name.
    pub id: u64,
    pub settings: Settings,
    /// Messages by arrival key, which grows with every message added.
    messages: BTreeMap<u64, Message>,
    /// `(SentAt, arrival key)`, to find expired messages without scanning.
    by_sent_at: BTreeSet<(u64, u64)>,
    next_key: u64,
    next_sequence: u64,
    dedup: Dedup,
}

impl Queue {
    pub fn new(id: u64, settings: Settings) -> Self {
        Self {
            id,
            settings,
            messages: BTreeMap::new(),
            by_sent_at: BTreeSet::new(),
            next_key: 0,
            next_sequence: 1,
            dedup: Dedup::default(),
        }
    }

    pub fn len(&self) -> usize {
        self.messages.len()
    }

    /// Removes every message that has outlived the retention period.
    pub fn expire(&mut self, now: u64) {
        let retention = u64::from(self.settings.retention_seconds);
        while let Some(&(sent_at, key)) = self.by_sent_at.first() {
            if now < sent_at + retention {
                break;
            }
            self.by_sent_at.pop_first();
            self.messages.remove(&key);
        }
    }

    pub fn purge(&mut self) {
        self.messages.clear();
        self.by_sent_at.clear();
    }

    pub fn counts(&self, now: u64) -> Counts {
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

    pub fn message(&self, key: u64) -> Option<&Message> {
        self.messages.get(&key)
    }

    pub fn message_mut(&mut self, key: u64) -> Option<&mut Message> {
        self.messages.get_mut(&key)
    }

    pub fn remove(&mut self, key: u64) -> Option<Message> {
        let message = self.messages.remove(&key)?;
        self.by_sent_at.remove(&(message.sent_at, key));
        Some(message)
    }

    /// Appends a message, giving it the next sequence number on a FIFO queue.
    pub fn push(&mut self, mut message: Message) -> (u64, Option<u64>) {
        let key = self.next_key;
        self.next_key += 1;
        if self.settings.fifo {
            message.sequence_number = Some(self.next_sequence);
            self.next_sequence += 1;
        }
        let sequence = message.sequence_number;
        self.by_sent_at.insert((message.sent_at, key));
        self.messages.insert(key, message);
        (key, sequence)
    }

    /// The message a send with this deduplication id duplicates, if any.
    pub fn duplicate_of(&self, dedup_id: &str, now: u64) -> Option<Original> {
        self.dedup.find(dedup_id, now).map(|entry| Original {
            message_id: entry.message_id,
            sequence_number: entry.sequence_number,
        })
    }

    pub fn remember_dedup_id(
        &mut self,
        dedup_id: &str,
        now: u64,
        message_id: u64,
        sequence_number: u64,
    ) {
        let entry = DedupEntry {
            at: now,
            message_id,
            sequence_number,
        };
        self.dedup.record(dedup_id, entry, now);
    }

    /// Adds a message that was dead-lettered from another queue.
    pub fn push_dead_letter(&mut self, mut message: Message, now: u64) {
        message.reset_for_dead_letter();
        self.push(message);
        self.expire(now);
    }

    /// Delivers up to `max` messages in arrival order and dead-letters the exhausted ones.
    /// Handles are numbered from `first_handle` in delivery order.
    pub fn receive(
        &mut self,
        now: u64,
        max: usize,
        visibility: u32,
        first_handle: u64,
    ) -> Received {
        let fifo = self.settings.fifo;
        let max_receive_count = self
            .settings
            .dead_letter
            .as_ref()
            .map(|d| d.max_receive_count);

        // Decide first, with only shared borrows; apply afterwards.
        let mut deliver = Vec::new();
        let mut dead = Vec::new();
        let mut groups_in_queue: HashSet<&str> = HashSet::new();
        for (&key, message) in &self.messages {
            if deliver.len() == max {
                break;
            }
            let group = message.group_id.as_deref().unwrap_or_default();
            // An earlier message of the group that stays in the queue blocks this one.
            if fifo && !groups_in_queue.insert(group) {
                continue;
            }
            if message.state(now) != State::Visible {
                continue;
            }
            if max_receive_count.is_some_and(|limit| message.receive_count >= limit) {
                // It leaves the queue, so it does not block its group.
                groups_in_queue.remove(group);
                dead.push(key);
            } else {
                deliver.push(key);
            }
        }

        let dead = dead
            .into_iter()
            .filter_map(|key| self.remove(key))
            .collect();
        let mut delivered = Vec::with_capacity(deliver.len());
        for (handle, key) in (first_handle..).zip(deliver) {
            let message = self.messages.get_mut(&key).expect("examined above");
            message.receive_count += 1;
            message.first_received_at.get_or_insert(now);
            message.in_flight_until = now + u64::from(visibility);
            message.latest_handle = Some(handle);
            delivered.push((key, message.clone()));
        }
        Received { delivered, dead }
    }
}

pub fn queue_does_not_exist(name: &str) -> Error {
    Error::new(
        ErrorCode::QueueDoesNotExist,
        format!("queue {name:?} does not exist"),
    )
}
