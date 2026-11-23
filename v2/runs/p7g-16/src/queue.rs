//! A single queue and its messages (SPEC section 4).

use std::collections::{BTreeMap, HashMap};

use crate::attributes::QueueConfig;

/// Seconds on the virtual clock.
pub type Time = u64;

/// How long a FIFO deduplication id stays reserved.
pub const DEDUPLICATION_WINDOW: Time = 300;

/// Maximum number of messages one queue may hold.
pub const MAX_MESSAGES: usize = 100_000;

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
    pub sent_at: Time,
    pub receive_count: u32,
    pub first_received_at: Option<Time>,
    pub group_id: Option<String>,
    /// Position in the queue's arrival order; also the FIFO `SequenceNumber`.
    pub sequence: u64,
    /// The message is delayed while `now` is below this.
    pub delayed_until: Time,
    /// The message is in flight while `now` is below this.
    pub in_flight_until: Time,
    /// Number of the latest receipt handle (`r-<n>`) issued for this message.
    pub receipt: Option<u64>,
}

impl Message {
    pub fn state(&self, now: Time) -> State {
        if now < self.delayed_until {
            State::Delayed
        } else if now < self.in_flight_until {
            State::InFlight
        } else {
            State::Visible
        }
    }
}

/// What a duplicate FIFO send answers with.
#[derive(Debug, Clone, Copy)]
pub struct Accepted {
    pub at: Time,
    pub message_id: u64,
    pub sequence: u64,
}

#[derive(Debug)]
pub struct Queue {
    pub config: QueueConfig,
    /// Messages keyed by arrival sequence, so iteration is arrival order.
    pub messages: BTreeMap<u64, Message>,
    /// Sequence number given to the next message added.
    next_sequence: u64,
    /// Receipt handle number -> sequence of the message it was issued for.
    /// May hold stale entries; always check the message's own `receipt`.
    pub handles: HashMap<u64, u64>,
    /// FIFO deduplication ids that were accepted recently.
    pub deduplication: HashMap<String, Accepted>,
}

impl Queue {
    pub fn new(config: QueueConfig) -> Self {
        Self {
            config,
            messages: BTreeMap::new(),
            next_sequence: 1,
            handles: HashMap::new(),
            deduplication: HashMap::new(),
        }
    }

    /// Appends a message, assigning its arrival sequence. Returns that sequence.
    pub fn push(&mut self, mut message: Message) -> u64 {
        message.sequence = self.next_sequence;
        self.next_sequence += 1;
        let sequence = message.sequence;
        self.messages.insert(sequence, message);
        sequence
    }

    /// Removes messages whose retention has run out, and deduplication ids whose
    /// window has closed.
    pub fn expire(&mut self, now: Time) {
        let retention = Time::from(self.config.retention_seconds);
        self.messages.retain(|_, m| now < m.sent_at + retention);
        self.deduplication
            .retain(|_, a| now - a.at < DEDUPLICATION_WINDOW);
    }
}
