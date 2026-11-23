//! A single queue: its attributes, messages and deduplication memory.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};

use crate::attributes::Attributes;

/// How long (seconds) a FIFO deduplication id suppresses repeated sends.
pub const DEDUPLICATION_WINDOW: u64 = 300;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Delayed,
    InFlight,
    Visible,
}

#[derive(Debug, Clone)]
pub struct Message {
    pub id: u64,
    pub body: String,
    pub sent_at: u64,
    pub receive_count: u64,
    pub first_received_at: Option<u64>,
    pub group_id: Option<String>,
    pub sequence_number: Option<u64>,
    /// Delayed while `now < delayed_until`.
    pub delayed_until: u64,
    /// In flight while `now < in_flight_until` (and not delayed).
    pub in_flight_until: u64,
    /// Number of the latest receipt handle issued for this message.
    pub receipt_handle: Option<u64>,
}

impl Message {
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

/// What `ReceiveMessage` does with an eligible message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Deliver,
    MoveToDeadLetter,
}

/// The message a duplicate send is answered with.
#[derive(Debug, Clone, Copy)]
struct Accepted {
    at: u64,
    message_id: u64,
    sequence_number: u64,
}

pub struct Queue {
    /// Unique over the process lifetime, so a re-created queue shares nothing
    /// with its predecessor, not even receipt handles.
    pub uid: u64,
    pub attributes: Attributes,
    /// Messages by arrival key; keys only grow, so key order is arrival order.
    messages: BTreeMap<u64, Message>,
    /// `(sent_at, key)` of every message, to expire the oldest first.
    by_sent_at: BTreeSet<(u64, u64)>,
    next_key: u64,
    next_sequence: u64,
    accepted: HashMap<String, Accepted>,
    /// `(time, id)` in acceptance order, to forget ids once their window ends.
    accepted_order: VecDeque<(u64, String)>,
}

impl Queue {
    pub fn new(uid: u64, attributes: Attributes) -> Self {
        Self {
            uid,
            attributes,
            messages: BTreeMap::new(),
            by_sent_at: BTreeSet::new(),
            next_key: 1,
            next_sequence: 1,
            accepted: HashMap::new(),
            accepted_order: VecDeque::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.messages.len()
    }

    pub fn get(&self, key: u64) -> Option<&Message> {
        self.messages.get(&key)
    }

    pub fn get_mut(&mut self, key: u64) -> Option<&mut Message> {
        self.messages.get_mut(&key)
    }

    /// Adds `message` at the end of the queue, assigning its sequence number
    /// on a FIFO queue. Returns the arrival key and the sequence number.
    pub fn push(&mut self, mut message: Message) -> (u64, Option<u64>) {
        message.sequence_number = self.attributes.fifo.then(|| {
            let n = self.next_sequence;
            self.next_sequence += 1;
            n
        });
        let key = self.next_key;
        self.next_key += 1;
        self.by_sent_at.insert((message.sent_at, key));
        let sequence = message.sequence_number;
        self.messages.insert(key, message);
        (key, sequence)
    }

    pub fn remove(&mut self, key: u64) -> Option<Message> {
        let message = self.messages.remove(&key)?;
        self.by_sent_at.remove(&(message.sent_at, key));
        Some(message)
    }

    pub fn clear(&mut self) {
        self.messages.clear();
        self.by_sent_at.clear();
    }

    /// Removes every message that retention has expired at `now`.
    pub fn expire(&mut self, now: u64) {
        let retention = self.attributes.retention_seconds;
        while let Some(&(sent_at, key)) = self.by_sent_at.first() {
            if now < sent_at + retention {
                break;
            }
            self.by_sent_at.pop_first();
            self.messages.remove(&key);
        }
    }

    /// Decides, without changing anything, what a `ReceiveMessage` for at most
    /// `max_messages` deliveries does to which message, in arrival order.
    ///
    /// On a FIFO queue a message is skipped while an earlier message of its
    /// group is still in the queue; a delivered message stays in the queue
    /// (in flight) and so blocks the rest of its group, while one moved to the
    /// dead-letter queue does not.
    pub fn plan_receive(&self, now: u64, max_messages: usize) -> Vec<(u64, Action)> {
        let mut plan = Vec::new();
        let mut delivered = 0;
        let mut blocked_groups: HashSet<&str> = HashSet::new();
        for (&key, message) in &self.messages {
            if let Some(group) = message.group_id.as_deref() {
                if blocked_groups.contains(group) {
                    continue;
                }
                if message.state(now) != State::Visible || !self.should_move_to_dead_letter(message)
                {
                    blocked_groups.insert(group);
                }
            }
            if message.state(now) != State::Visible {
                continue;
            }
            if self.should_move_to_dead_letter(message) {
                plan.push((key, Action::MoveToDeadLetter));
            } else {
                plan.push((key, Action::Deliver));
                delivered += 1;
                if delivered == max_messages {
                    break;
                }
            }
        }
        plan
    }

    fn should_move_to_dead_letter(&self, message: &Message) -> bool {
        self.attributes
            .dead_letter
            .as_ref()
            .is_some_and(|d| message.receive_count >= d.max_receive_count)
    }

    /// Number of messages in each state: `(visible, in_flight, delayed)`.
    pub fn counts(&self, now: u64) -> (usize, usize, usize) {
        let mut counts = (0, 0, 0);
        for message in self.messages.values() {
            match message.state(now) {
                State::Visible => counts.0 += 1,
                State::InFlight => counts.1 += 1,
                State::Delayed => counts.2 += 1,
            }
        }
        counts
    }

    /// The original message of a send that repeats `deduplication_id` within
    /// the window, as `(message_id, sequence_number)`.
    pub fn duplicate_of(&self, deduplication_id: &str, now: u64) -> Option<(u64, u64)> {
        let a = self.accepted.get(deduplication_id)?;
        (now - a.at < DEDUPLICATION_WINDOW).then_some((a.message_id, a.sequence_number))
    }

    /// Remembers an accepted FIFO send, opening a new window for its id.
    pub fn remember(
        &mut self,
        deduplication_id: String,
        now: u64,
        message_id: u64,
        sequence_number: u64,
    ) {
        while let Some((at, id)) = self.accepted_order.front() {
            if now - at < DEDUPLICATION_WINDOW {
                break;
            }
            // A newer window for the same id must survive.
            if self.accepted.get(id).is_some_and(|a| a.at == *at) {
                self.accepted.remove(id);
            }
            self.accepted_order.pop_front();
        }
        self.accepted_order
            .push_back((now, deduplication_id.clone()));
        self.accepted.insert(
            deduplication_id,
            Accepted {
                at: now,
                message_id,
                sequence_number,
            },
        );
    }
}
