//! A single queue: its attributes and its messages.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::fmt;
use std::ops::Bound;

use crate::command::AttributeUpdate;

pub const MAX_MESSAGES_PER_QUEUE: usize = 100_000;

/// Process-wide delivery number; rendered as `r-<n>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ReceiptHandle(pub u64);

impl fmt::Display for ReceiptHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "r-{}", self.0)
    }
}

/// How long (seconds) a FIFO queue remembers a deduplication id.
const DEDUP_WINDOW: u64 = 300;

/// Process-wide message number; rendered as `m-<n>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MessageId(pub u64);

impl fmt::Display for MessageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "m-{}", self.0)
    }
}

const FIFO_SUFFIX: &str = ".fifo";

pub fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

/// Name syntax of SPEC section 4: 1 to 80 name characters, or 1 to 75 of
/// them followed by `.fifo`.
pub fn is_valid_queue_name(name: &str) -> bool {
    let stem = name.strip_suffix(FIFO_SUFFIX).unwrap_or(name);
    let max = if stem.len() == name.len() { 80 } else { 75 };
    (1..=max).contains(&stem.len()) && stem.chars().all(is_name_char)
}

pub fn has_fifo_suffix(name: &str) -> bool {
    name.ends_with(FIFO_SUFFIX)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeadLetterConfig {
    pub queue: String,
    pub max_receive_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attributes {
    pub visibility_timeout: u32,
    pub delay_seconds: u32,
    pub retention_seconds: u64,
    pub fifo: bool,
    pub content_based_deduplication: bool,
    pub dead_letter: Option<DeadLetterConfig>,
}

impl Default for Attributes {
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

impl Attributes {
    /// The attributes of a new queue: given values, defaults for the rest.
    pub fn from_update(update: &AttributeUpdate) -> Self {
        let mut attributes = Self {
            fifo: update.fifo.unwrap_or(false),
            ..Self::default()
        };
        attributes.apply(update);
        attributes
    }

    /// Overwrites the attributes that `update` mentions.
    pub fn apply(&mut self, update: &AttributeUpdate) {
        if let Some(v) = update.visibility_timeout {
            self.visibility_timeout = v;
        }
        if let Some(v) = update.delay_seconds {
            self.delay_seconds = v;
        }
        if let Some(v) = update.retention_seconds {
            self.retention_seconds = v;
        }
        if let Some(v) = update.content_based_deduplication {
            self.content_based_deduplication = v;
        }
        // The command layer guarantees both or neither are present.
        if let (Some(queue), Some(max_receive_count)) =
            (&update.dead_letter_queue, update.max_receive_count)
        {
            self.dead_letter = Some(DeadLetterConfig {
                queue: queue.clone(),
                max_receive_count,
            });
        }
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
    pub id: MessageId,
    pub body: String,
    pub sent_at: u64,
    pub receive_count: u32,
    pub first_received_at: Option<u64>,
    pub group_id: Option<String>,
    pub sequence_number: Option<u64>,
    /// The message is delayed while `now < delayed_until`.
    pub delayed_until: u64,
    /// The message is in flight while `now < in_flight_until`.
    pub in_flight_until: u64,
    /// Number of the latest receipt handle issued for this message.
    pub receipt_handle: Option<ReceiptHandle>,
}

impl Message {
    /// Returns the message to the state it has when first added to a queue.
    fn reset_for_dead_letter(&mut self) {
        self.receive_count = 0;
        self.first_received_at = None;
        self.delayed_until = 0;
        self.in_flight_until = 0;
        self.receipt_handle = None;
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
}

#[derive(Debug, Default)]
pub struct Received {
    pub delivered: Vec<Message>,
    pub dead: Vec<Message>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Counts {
    pub visible: usize,
    pub in_flight: usize,
    pub delayed: usize,
}

#[derive(Debug)]
pub struct Queue {
    pub attributes: Attributes,
    /// Messages keyed by arrival number, which gives arrival order.
    messages: BTreeMap<u64, Message>,
    next_arrival: u64,
    /// `(sent_at, arrival)` of every message, so expiry never scans the queue.
    by_sent_at: BTreeSet<(u64, u64)>,
    /// Receipt handle number to the arrival number of the message it was issued for.
    handles: HashMap<ReceiptHandle, u64>,
    next_sequence_number: u64,
    /// Deduplication id to the message it was first used for.
    dedup: HashMap<String, DedupEntry>,
    /// Deduplication ids in the order they were recorded, to forget old ones.
    dedup_order: VecDeque<(u64, String)>,
}

#[derive(Debug, Clone, Copy)]
struct DedupEntry {
    accepted_at: u64,
    message: MessageId,
    sequence_number: Option<u64>,
}

/// What a duplicate send reports: the ids of the original message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Original {
    pub message: MessageId,
    pub sequence_number: Option<u64>,
}

impl Queue {
    pub fn new(attributes: Attributes) -> Self {
        Self {
            attributes,
            messages: BTreeMap::new(),
            next_arrival: 0,
            by_sent_at: BTreeSet::new(),
            handles: HashMap::new(),
            next_sequence_number: 1,
            dedup: HashMap::new(),
            dedup_order: VecDeque::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.messages.len()
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

    /// Removes every message whose retention period is over at `now`.
    ///
    /// Must be called before a queue is read or its retention is changed, so
    /// that expiry always uses the retention that was in force.
    pub fn expire(&mut self, now: u64) {
        while let Some(&(sent_at, arrival)) = self.by_sent_at.first() {
            if now < sent_at + self.attributes.retention_seconds {
                break;
            }
            self.remove(arrival);
        }
    }

    /// Adds a message at the end of the queue. On a FIFO queue it is given
    /// the next sequence number, which is returned.
    pub fn push(&mut self, mut message: Message) -> Option<u64> {
        let sequence_number = self.attributes.fifo.then(|| {
            let n = self.next_sequence_number;
            self.next_sequence_number += 1;
            n
        });
        message.sequence_number = sequence_number;
        let arrival = self.next_arrival;
        self.next_arrival += 1;
        self.by_sent_at.insert((message.sent_at, arrival));
        self.messages.insert(arrival, message);
        sequence_number
    }

    /// The message that used `dedup_id` less than five minutes before `now`.
    pub fn original_of(&mut self, dedup_id: &str, now: u64) -> Option<Original> {
        self.forget_old_dedup_ids(now);
        self.dedup.get(dedup_id).map(|entry| Original {
            message: entry.message,
            sequence_number: entry.sequence_number,
        })
    }

    /// Remembers that `dedup_id` was used by an accepted message at `now`.
    pub fn record_dedup_id(&mut self, dedup_id: String, now: u64, original: Original) {
        self.dedup_order.push_back((now, dedup_id.clone()));
        self.dedup.insert(
            dedup_id,
            DedupEntry {
                accepted_at: now,
                message: original.message,
                sequence_number: original.sequence_number,
            },
        );
    }

    fn forget_old_dedup_ids(&mut self, now: u64) {
        while let Some((recorded_at, dedup_id)) = self.dedup_order.front() {
            if now - recorded_at < DEDUP_WINDOW {
                break;
            }
            // The id may have been recorded again after its window ended.
            if self
                .dedup
                .get(dedup_id)
                .is_some_and(|entry| entry.accepted_at == *recorded_at)
            {
                self.dedup.remove(dedup_id);
            }
            self.dedup_order.pop_front();
        }
    }

    pub fn remove(&mut self, arrival: u64) -> Option<Message> {
        let message = self.messages.remove(&arrival)?;
        self.by_sent_at.remove(&(message.sent_at, arrival));
        if let Some(handle) = message.receipt_handle {
            self.handles.remove(&handle);
        }
        Some(message)
    }

    /// Examines the messages once each in arrival order (SPEC section 6),
    /// delivering up to `max` of them. Messages that have reached the
    /// maximum receive count are removed and returned as `dead`; the caller
    /// adds them to the dead-letter queue.
    pub fn receive(
        &mut self,
        now: u64,
        max: usize,
        visibility_timeout: u32,
        last_handle: &mut u64,
    ) -> Received {
        let mut received = Received::default();
        // Groups that still have an earlier message in this queue.
        let mut occupied_groups: HashSet<String> = HashSet::new();
        let mut after = Bound::Unbounded;
        while received.delivered.len() < max {
            let Some((&arrival, message)) =
                self.messages.range_mut((after, Bound::Unbounded)).next()
            else {
                break;
            };
            after = Bound::Excluded(arrival);
            let group_taken = message
                .group_id
                .as_ref()
                .is_some_and(|group| occupied_groups.contains(group));
            let eligible = !group_taken && message.state(now) == State::Visible;
            let dead = eligible
                && self
                    .attributes
                    .dead_letter
                    .as_ref()
                    .is_some_and(|config| message.receive_count >= config.max_receive_count);
            if dead {
                if let Some(message) = self.remove(arrival) {
                    received.dead.push(message);
                }
                continue;
            }
            if let Some(group) = &message.group_id {
                if !group_taken {
                    occupied_groups.insert(group.clone());
                }
            }
            if !eligible {
                continue;
            }
            if let Some(old) = message.receipt_handle {
                self.handles.remove(&old);
            }
            *last_handle += 1;
            let handle = ReceiptHandle(*last_handle);
            message.receive_count += 1;
            message.first_received_at.get_or_insert(now);
            message.receipt_handle = Some(handle);
            message.in_flight_until = now + u64::from(visibility_timeout);
            self.handles.insert(handle, arrival);
            received.delivered.push(message.clone());
        }
        for message in &mut received.dead {
            message.reset_for_dead_letter();
        }
        received
    }

    /// The arrival number of the message `handle` is the latest handle of,
    /// provided that message is in flight.
    fn in_flight_with_handle(&self, handle: ReceiptHandle, now: u64) -> Option<u64> {
        let arrival = *self.handles.get(&handle)?;
        let message = self.messages.get(&arrival)?;
        (message.receipt_handle == Some(handle) && message.state(now) == State::InFlight)
            .then_some(arrival)
    }

    /// Deletes the message `handle` was issued for; false if the handle is
    /// not usable.
    pub fn delete_by_handle(&mut self, handle: ReceiptHandle, now: u64) -> bool {
        match self.in_flight_with_handle(handle, now) {
            Some(arrival) => self.remove(arrival).is_some(),
            None => false,
        }
    }

    /// Makes the message `handle` was issued for in flight until
    /// `now + visibility_timeout`; false if the handle is not usable.
    pub fn change_visibility(
        &mut self,
        handle: ReceiptHandle,
        now: u64,
        visibility_timeout: u32,
    ) -> bool {
        let Some(arrival) = self.in_flight_with_handle(handle, now) else {
            return false;
        };
        match self.messages.get_mut(&arrival) {
            Some(message) => {
                message.in_flight_until = now + u64::from(visibility_timeout);
                true
            }
            None => false,
        }
    }

    pub fn purge(&mut self) {
        self.messages.clear();
        self.by_sent_at.clear();
        self.handles.clear();
    }
}
