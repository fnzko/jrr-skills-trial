//! A single queue: its attributes and its messages.

use std::collections::{BTreeMap, HashMap, HashSet};

pub const MAX_QUEUES: usize = 1000;
pub const MAX_MESSAGES_PER_QUEUE: usize = 100_000;

/// Seconds during which a FIFO queue remembers a deduplication id.
const DEDUPLICATION_WINDOW: u64 = 300;

const DEFAULT_VISIBILITY_TIMEOUT: u32 = 30;
const DEFAULT_RETENTION_SECONDS: u32 = 345_600;

/// Identifies one incarnation of a queue; a queue created again under a
/// deleted name gets a new id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct QueueId(pub u64);

/// Where messages go after too many deliveries. Both parts always exist together.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeadLetter {
    pub queue: String,
    pub max_receive_count: u32,
}

/// The attributes of a queue except `Fifo`, which never changes and is
/// recorded by [`Queue::fifo`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub visibility_timeout: u32,
    pub delay_seconds: u32,
    pub retention_seconds: u32,
    pub content_based_deduplication: bool,
    pub dead_letter: Option<DeadLetter>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            visibility_timeout: DEFAULT_VISIBILITY_TIMEOUT,
            delay_seconds: 0,
            retention_seconds: DEFAULT_RETENTION_SECONDS,
            content_based_deduplication: false,
            dead_letter: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Delayed,
    InFlight,
    Visible,
}

/// The latest delivery of a message.
#[derive(Debug, Clone, Copy)]
pub struct Delivery {
    pub handle: u64,
    /// The message is in flight while `now < invisible_until`.
    pub invisible_until: u64,
}

#[derive(Debug, Clone)]
pub struct Message {
    pub id: u64,
    pub body: String,
    pub sent_at: u64,
    /// The message is delayed while `now < delayed_until`.
    pub delayed_until: u64,
    pub receive_count: u64,
    pub first_received_at: Option<u64>,
    pub delivery: Option<Delivery>,
    /// Set on FIFO queues only.
    pub group: Option<String>,
    /// Set by the FIFO queue that holds the message.
    pub sequence: Option<u64>,
}

impl Message {
    pub fn state(&self, now: u64) -> State {
        if now < self.delayed_until {
            State::Delayed
        } else if self.delivery.is_some_and(|d| now < d.invisible_until) {
            State::InFlight
        } else {
            State::Visible
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Counts {
    pub visible: usize,
    pub in_flight: usize,
    pub delayed: usize,
}

/// The message accepted for a deduplication id, and when.
#[derive(Debug)]
struct Accepted {
    at: u64,
    message_id: u64,
    sequence: u64,
}

#[derive(Debug)]
struct FifoState {
    next_sequence: u64,
    /// Outlives the messages: an id stays here after its message is gone.
    deduplication: HashMap<String, Accepted>,
}

/// The identity of an accepted message, as returned to the sender.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sent {
    pub message_id: u64,
    pub sequence: Option<u64>,
}

/// What a `ReceiveMessage` did to a queue.
#[derive(Debug, Default)]
pub struct Received {
    /// Copies of the delivered messages, in delivery order, each with its
    /// `delivery` set.
    pub delivered: Vec<Message>,
    /// Messages that left the queue for its dead-letter queue.
    pub dead: Vec<Message>,
}

/// Issues receipt handles and remembers where each one was issued.
#[derive(Debug, Default)]
pub struct Receipts {
    issued: u64,
    /// Handle number to the queue and arrival key of its message.
    locations: HashMap<u64, (QueueId, u64)>,
}

impl Receipts {
    fn issue(&mut self, queue: QueueId, key: u64) -> u64 {
        self.issued += 1;
        self.locations.insert(self.issued, (queue, key));
        self.issued
    }

    /// The number of a handle string that this process has issued. Only the
    /// canonical spelling `r-<n>` counts, so `r-01` and `r-1 ` do not.
    pub fn parse(&self, text: &str) -> Option<u64> {
        let digits = text.strip_prefix("r-")?;
        if digits.starts_with('0') || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        digits
            .parse()
            .ok()
            .filter(|n| (1..=self.issued).contains(n))
    }

    pub fn location(&self, handle: u64) -> Option<(QueueId, u64)> {
        self.locations.get(&handle).copied()
    }
}

#[derive(Debug)]
pub struct Queue {
    pub id: QueueId,
    pub config: Config,
    fifo: Option<FifoState>,
    /// In arrival order. The key is a per-queue arrival counter, so a message
    /// moved here from another queue lands at the end whatever its `MessageId`.
    messages: BTreeMap<u64, Message>,
    next_arrival: u64,
    /// A lower bound of the `sent_at` of all messages, `u64::MAX` when empty.
    /// It lets [`Queue::sweep`] skip the scan until something can expire.
    oldest_sent_at: u64,
}

impl Queue {
    pub fn new(id: QueueId, fifo: bool, config: Config) -> Self {
        Self {
            id,
            config,
            fifo: fifo.then(|| FifoState {
                next_sequence: 1,
                deduplication: HashMap::new(),
            }),
            messages: BTreeMap::new(),
            next_arrival: 0,
            oldest_sent_at: u64::MAX,
        }
    }

    pub fn is_fifo(&self) -> bool {
        self.fifo.is_some()
    }

    pub fn len(&self) -> usize {
        self.messages.len()
    }

    /// Removes the messages whose retention has run out. Every command that
    /// reads or changes a queue calls this first, which makes retention hold
    /// at every moment without a timer.
    pub fn sweep(&mut self, now: u64) {
        let retention = u64::from(self.config.retention_seconds);
        if now < self.oldest_sent_at.saturating_add(retention) {
            return;
        }
        self.messages
            .retain(|_, m| now < m.sent_at.saturating_add(retention));
        self.oldest_sent_at = self
            .messages
            .values()
            .map(|m| m.sent_at)
            .min()
            .unwrap_or(u64::MAX);
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
        self.oldest_sent_at = u64::MAX;
    }

    /// The original message when `id` was accepted less than the
    /// deduplication window ago.
    pub fn find_duplicate(&self, id: &str, now: u64) -> Option<Sent> {
        let accepted = self.fifo.as_ref()?.deduplication.get(id)?;
        (now - accepted.at < DEDUPLICATION_WINDOW).then_some(Sent {
            message_id: accepted.message_id,
            sequence: Some(accepted.sequence),
        })
    }

    /// Adds a message at the end and records its deduplication id, if any.
    /// The caller has checked the limits.
    pub fn send(&mut self, message: Message, deduplication_id: Option<String>, now: u64) -> Sent {
        let message_id = message.id;
        let sent = self.add(message);
        if let (Some(fifo), Some(id), Some(sequence)) =
            (self.fifo.as_mut(), deduplication_id, sent.sequence)
        {
            fifo.deduplication.insert(
                id,
                Accepted {
                    at: now,
                    message_id,
                    sequence,
                },
            );
        }
        sent
    }

    /// Adds a message that another queue gave up on. It starts over: no
    /// receives, no delay, and a new sequence number. Retention keeps
    /// counting from `sent_at`, so it may be gone on arrival.
    pub fn add_dead_letter(&mut self, mut message: Message, now: u64) {
        message.receive_count = 0;
        message.first_received_at = None;
        message.delayed_until = message.sent_at;
        message.delivery = None;
        self.add(message);
        self.sweep(now);
    }

    fn add(&mut self, mut message: Message) -> Sent {
        message.sequence = self.fifo.as_mut().map(|fifo| {
            let sequence = fifo.next_sequence;
            fifo.next_sequence += 1;
            sequence
        });
        let sent = Sent {
            message_id: message.id,
            sequence: message.sequence,
        };
        self.oldest_sent_at = self.oldest_sent_at.min(message.sent_at);
        self.messages.insert(self.next_arrival, message);
        self.next_arrival += 1;
        sent
    }

    /// Delivers up to `max_messages` messages. `visibility_timeout` is for
    /// this call only; the queue's own attribute is not touched.
    pub fn receive(
        &mut self,
        now: u64,
        max_messages: usize,
        visibility_timeout: u32,
        receipts: &mut Receipts,
    ) -> Received {
        let max_receive_count = self
            .config
            .dead_letter
            .as_ref()
            .map(|d| u64::from(d.max_receive_count));
        // Groups that still have a message in the queue, ahead of the one
        // being examined.
        let mut busy_groups: HashSet<String> = HashSet::new();
        let mut received = Received::default();
        let mut dead_keys = Vec::new();

        for (&key, message) in &mut self.messages {
            if received.delivered.len() == max_messages {
                break;
            }
            let group_busy = message
                .group
                .as_deref()
                .is_some_and(|group| busy_groups.contains(group));
            let eligible = !group_busy && message.state(now) == State::Visible;
            if eligible && max_receive_count.is_some_and(|max| message.receive_count >= max) {
                // Leaving the queue, so it does not hold back its group.
                dead_keys.push(key);
                continue;
            }
            if eligible {
                message.receive_count += 1;
                message.first_received_at.get_or_insert(now);
                message.delivery = Some(Delivery {
                    handle: receipts.issue(self.id, key),
                    invisible_until: now + u64::from(visibility_timeout),
                });
                received.delivered.push(message.clone());
            }
            if let Some(group) = message.group.as_deref()
                && !busy_groups.contains(group)
            {
                busy_groups.insert(group.to_owned());
            }
        }

        received.dead = dead_keys
            .into_iter()
            .map(|key| {
                self.messages
                    .remove(&key)
                    .expect("keys were collected from this map")
            })
            .collect();
        received
    }

    fn in_flight_mut(&mut self, key: u64, handle: u64, now: u64) -> Option<&mut Message> {
        self.messages.get_mut(&key).filter(|message| {
            message.state(now) == State::InFlight
                && message.delivery.is_some_and(|d| d.handle == handle)
        })
    }

    /// Removes the message if `handle` is the latest handle of a message that
    /// is in flight at `now`. Returns whether it did.
    pub fn delete_in_flight(&mut self, key: u64, handle: u64, now: u64) -> bool {
        let found = self.in_flight_mut(key, handle, now).is_some();
        if found {
            self.messages.remove(&key);
        }
        found
    }

    /// Ends the visibility timeout of an in-flight message `seconds` after
    /// `now`. Returns whether `handle` was valid, as for `delete_in_flight`.
    pub fn change_visibility(&mut self, key: u64, handle: u64, now: u64, seconds: u32) -> bool {
        match self
            .in_flight_mut(key, handle, now)
            .and_then(|message| message.delivery.as_mut())
        {
            Some(delivery) => {
                delivery.invisible_until = now + u64::from(seconds);
                true
            }
            None => false,
        }
    }
}
