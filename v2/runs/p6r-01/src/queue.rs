//! A single queue: its attributes, messages and bookkeeping.
//!
//! The queue knows nothing about other queues or the clock's owner; the
//! broker passes `now` in and coordinates dead-letter moves.

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::attributes::{AttributeUpdate, Attributes};
use crate::ids::{Arrival, MessageId, ReceiptHandle};

pub const MAX_MESSAGES: usize = 100_000;
const DEDUP_WINDOW: u64 = 300;
const MIN_DEDUP_PRUNE_LEN: usize = 1024;

#[derive(Clone, Debug)]
pub struct Message {
    pub id: MessageId,
    pub body: String,
    pub sent_at: u64,
    pub receive_count: u64,
    pub first_received_at: Option<u64>,
    /// Set exactly for messages of a FIFO queue.
    pub fifo: Option<FifoPosition>,
    /// The message is delayed while `now < delayed_until`.
    pub delayed_until: u64,
    /// The message is in flight while `now < invisible_until`.
    pub invisible_until: u64,
    /// The latest receipt handle issued for this message.
    pub handle: Option<ReceiptHandle>,
}

/// Where a message of a FIFO queue belongs: its group and its place in the
/// queue's sequence.
#[derive(Clone, Debug)]
pub struct FifoPosition {
    pub group_id: String,
    pub sequence_number: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Delayed,
    InFlight,
    Visible,
}

impl Message {
    pub fn state(&self, now: u64) -> State {
        if now < self.delayed_until {
            State::Delayed
        } else if now < self.invisible_until {
            State::InFlight
        } else {
            State::Visible
        }
    }
}

/// What `ReceiveMessage` does with one examined message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Deliver(Arrival),
    DeadLetter(Arrival),
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Counts {
    pub visible: u64,
    pub in_flight: u64,
    pub delayed: u64,
}

/// The original message of a deduplication id.
#[derive(Clone, Copy, Debug)]
pub struct Accepted {
    pub accepted_at: u64,
    pub message_id: MessageId,
    pub sequence_number: u64,
}

pub struct Queue {
    attributes: Attributes,
    /// Messages in arrival order, keyed by an ever-growing arrival index.
    messages: BTreeMap<Arrival, Message>,
    next_arrival: Arrival,
    /// Latest receipt handle -> arrival index of its message.
    handles: HashMap<ReceiptHandle, Arrival>,
    next_sequence: u64,
    deduplication: HashMap<String, Accepted>,
    prune_deduplication_at: usize,
    /// A lower bound of the `sent_at` of all messages, to skip needless sweeps.
    oldest_sent_at: Option<u64>,
}

impl Queue {
    pub fn new(attributes: Attributes) -> Self {
        Self {
            attributes,
            messages: BTreeMap::new(),
            next_arrival: Arrival::first(),
            handles: HashMap::new(),
            next_sequence: 1,
            deduplication: HashMap::new(),
            prune_deduplication_at: MIN_DEDUP_PRUNE_LEN,
            oldest_sent_at: None,
        }
    }

    pub fn attributes(&self) -> &Attributes {
        &self.attributes
    }

    /// Applies an already validated update. A new `RetentionSeconds` takes
    /// effect on all messages at once, so expiry runs right away.
    pub fn apply_attributes(&mut self, update: AttributeUpdate, now: u64) {
        self.attributes.apply(update);
        self.expire(now);
    }

    /// Unsets the dead-letter configuration if it names `queue`.
    pub fn forget_dead_letter_queue(&mut self, queue: &str) {
        if self
            .attributes
            .dead_letter
            .as_ref()
            .is_some_and(|dl| dl.queue == queue)
        {
            self.attributes.dead_letter = None;
        }
    }

    pub fn len(&self) -> usize {
        self.messages.len()
    }

    /// Removes the messages whose retention has run out.
    pub fn expire(&mut self, now: u64) {
        let retention = self.attributes.retention_seconds;
        let is_expired = |m: &Message| now >= m.sent_at + retention;
        if !self.oldest_sent_at.is_some_and(|t| now >= t + retention) {
            return;
        }
        let expired: Vec<Arrival> = self
            .messages
            .iter()
            .filter(|(_, m)| is_expired(m))
            .map(|(&key, _)| key)
            .collect();
        for key in expired {
            self.remove(key);
        }
        self.oldest_sent_at = self.messages.values().map(|m| m.sent_at).min();
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

    /// Removes every message. Sequence numbers and deduplication carry on.
    pub fn purge(&mut self) {
        self.messages.clear();
        self.handles.clear();
        self.oldest_sent_at = None;
    }

    /// Adds a message at the end of the queue and returns its sequence number
    /// if this is a FIFO queue. `group_id` is given exactly for FIFO queues
    /// and `message.fifo` is overwritten from it.
    pub fn push(&mut self, mut message: Message, group_id: Option<String>) -> Option<u64> {
        message.fifo = group_id.map(|group_id| {
            let sequence_number = self.next_sequence;
            self.next_sequence += 1;
            FifoPosition {
                group_id,
                sequence_number,
            }
        });
        let sequence = message.fifo.as_ref().map(|fifo| fifo.sequence_number);
        self.oldest_sent_at = Some(
            self.oldest_sent_at
                .map_or(message.sent_at, |t| t.min(message.sent_at)),
        );
        self.messages.insert(self.next_arrival.take_next(), message);
        sequence
    }

    /// Adds a message moved here from another queue's dead-letter handling.
    pub fn push_dead_letter(&mut self, mut message: Message, now: u64) {
        message.receive_count = 0;
        message.first_received_at = None;
        message.delayed_until = 0;
        message.invisible_until = 0;
        message.handle = None;
        let group_id = message.fifo.take().map(|fifo| fifo.group_id);
        self.push(message, group_id);
        self.expire(now);
    }

    /// Removes and returns the message with this arrival index.
    pub fn remove(&mut self, key: Arrival) -> Option<Message> {
        let message = self.messages.remove(&key)?;
        if let Some(handle) = message.handle {
            self.handles.remove(&handle);
        }
        Some(message)
    }

    /// The arrival index of the message that is in flight right now and whose
    /// latest handle is `handle`.
    pub fn find_in_flight(&self, handle: ReceiptHandle, now: u64) -> Option<Arrival> {
        let key = *self.handles.get(&handle)?;
        let message = self.messages.get(&key)?;
        (message.state(now) == State::InFlight).then_some(key)
    }

    pub fn message_mut(&mut self, key: Arrival) -> Option<&mut Message> {
        self.messages.get_mut(&key)
    }

    /// Examines the messages as `ReceiveMessage` does (SPEC section 6) and
    /// says what to do with each one; it changes nothing.
    pub fn plan_receive(&self, now: u64, max_messages: usize) -> Vec<Step> {
        let mut steps = Vec::new();
        let mut delivered = 0;
        // Groups that still have an earlier message in the queue.
        let mut blocked: HashSet<&str> = HashSet::new();
        for (&key, message) in &self.messages {
            let group = message.fifo.as_ref().map(|fifo| fifo.group_id.as_str());
            if group.is_some_and(|g| blocked.contains(g)) {
                continue;
            }
            if message.state(now) != State::Visible {
                blocked.extend(group);
                continue;
            }
            match &self.attributes.dead_letter {
                Some(dl) if message.receive_count >= dl.max_receive_count => {
                    steps.push(Step::DeadLetter(key));
                }
                _ => {
                    steps.push(Step::Deliver(key));
                    blocked.extend(group);
                    delivered += 1;
                    if delivered == max_messages {
                        break;
                    }
                }
            }
        }
        steps
    }

    /// Delivers a message: it is now in flight for `visibility_timeout`
    /// seconds under the new receipt `handle`.
    pub fn deliver(
        &mut self,
        key: Arrival,
        now: u64,
        visibility_timeout: u64,
        handle: ReceiptHandle,
    ) -> &Message {
        let message = self.messages.get_mut(&key).expect("planned message exists");
        if let Some(old) = message.handle.replace(handle) {
            self.handles.remove(&old);
        }
        self.handles.insert(handle, key);
        message.receive_count += 1;
        message.first_received_at.get_or_insert(now);
        message.invisible_until = now + visibility_timeout;
        message
    }

    /// The original send of this deduplication id, if its window is open.
    pub fn duplicate_of(&self, deduplication_id: &str, now: u64) -> Option<Accepted> {
        self.deduplication
            .get(deduplication_id)
            .filter(|a| now - a.accepted_at < DEDUP_WINDOW)
            .copied()
    }

    /// Opens a deduplication window for an accepted send.
    pub fn record_deduplication(&mut self, deduplication_id: &str, accepted: Accepted) {
        if self.deduplication.len() >= self.prune_deduplication_at {
            let now = accepted.accepted_at;
            self.deduplication
                .retain(|_, a| now - a.accepted_at < DEDUP_WINDOW);
            self.prune_deduplication_at = (self.deduplication.len() * 2).max(MIN_DEDUP_PRUNE_LEN);
        }
        self.deduplication
            .insert(deduplication_id.to_owned(), accepted);
    }
}
