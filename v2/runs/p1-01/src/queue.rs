//! A queue, its attributes and its messages.
//!
//! All time here is the virtual clock in seconds, passed in as `now`.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;

use serde::{Serialize, Serializer};

use crate::receipt::{DeliveryLog, ReceiptHandle};
use crate::request::{AttributeUpdate, DeadLetter};

/// A message that arrives is a duplicate of an earlier one with the same id for this long.
const DEDUPLICATION_WINDOW: u64 = 300;

/// Identifies one incarnation of a queue: a queue created again under the same name is new.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueueId(u64);

impl QueueId {
    pub fn new(number: u64) -> Self {
        Self(number)
    }
}

/// `m-<n>`, where `n` counts the messages accepted by the process from 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MessageId(u64);

impl MessageId {
    pub fn new(number: u64) -> Self {
        Self(number)
    }
}

impl fmt::Display for MessageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "m-{}", self.0)
    }
}

impl Serialize for MessageId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueConfig {
    pub fifo: bool,
    pub visibility_timeout: u64,
    pub delay_seconds: u64,
    pub retention_seconds: u64,
    pub content_based_deduplication: bool,
    pub dead_letter: Option<DeadLetter>,
}

impl QueueConfig {
    /// The defaults with the attributes of a `CreateQueue` request applied.
    pub fn new(update: &AttributeUpdate, dead_letter: Option<DeadLetter>) -> Self {
        let mut config = Self {
            fifo: update.fifo.unwrap_or(false),
            visibility_timeout: 30,
            delay_seconds: 0,
            retention_seconds: 345_600,
            content_based_deduplication: false,
            dead_letter: None,
        };
        config.apply(update, dead_letter);
        config
    }

    /// Overwrites the attributes present in `update`. `Fifo` never changes after creation.
    pub fn apply(&mut self, update: &AttributeUpdate, dead_letter: Option<DeadLetter>) {
        if let Some(value) = update.visibility_timeout {
            self.visibility_timeout = value;
        }
        if let Some(value) = update.delay_seconds {
            self.delay_seconds = value;
        }
        if let Some(value) = update.retention_seconds {
            self.retention_seconds = value;
        }
        if let Some(value) = update.content_based_deduplication {
            self.content_based_deduplication = value;
        }
        if dead_letter.is_some() {
            self.dead_letter = dead_letter;
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Hold {
    Delayed { until: u64 },
    InFlight { until: u64, handle: ReceiptHandle },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Visible,
    InFlight,
    Delayed,
}

#[derive(Debug)]
pub struct Message {
    id: MessageId,
    body: String,
    sent_at: u64,
    receive_count: u64,
    first_received_at: Option<u64>,
    group_id: Option<String>,
    /// The latest reason the message is hidden. It may have run out; see [`Message::state`].
    hold: Option<Hold>,
}

impl Message {
    fn state(&self, now: u64) -> State {
        match self.hold {
            Some(Hold::Delayed { until }) if now < until => State::Delayed,
            Some(Hold::InFlight { until, .. }) if now < until => State::InFlight,
            _ => State::Visible,
        }
    }

    fn in_flight_handle(&self, now: u64) -> Option<ReceiptHandle> {
        match self.hold {
            Some(Hold::InFlight { until, handle }) if now < until => Some(handle),
            _ => None,
        }
    }
}

/// What the sender of a message to a FIFO queue supplies.
pub struct FifoSend {
    pub group_id: String,
    pub deduplication_id: String,
}

pub struct NewMessage {
    pub id: MessageId,
    pub body: String,
    pub delay_seconds: u64,
    pub fifo: Option<FifoSend>,
}

/// The message a duplicate send resolves to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Original {
    pub id: MessageId,
    pub sequence: u64,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub struct Counts {
    pub visible: u64,
    pub in_flight: u64,
    pub delayed: u64,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub struct Delivered {
    message_id: MessageId,
    receipt_handle: ReceiptHandle,
    body: String,
    receive_count: u64,
    sent_at: u64,
    first_received_at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    group_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sequence_number: Option<u64>,
}

/// The outcome of one `ReceiveMessage` on a queue.
pub struct Received {
    pub delivered: Vec<Delivered>,
    /// Messages that exceeded `MaxReceiveCount`, in the order they left the queue.
    pub dead: Vec<Message>,
}

enum Action {
    Deliver(u64),
    DeadLetter(u64),
}

#[derive(Debug)]
pub struct Queue {
    id: QueueId,
    config: QueueConfig,
    /// Keyed by sequence number, which is also the arrival order.
    messages: BTreeMap<u64, Message>,
    next_sequence: u64,
    /// Deduplication id to the send that opened its current window and when.
    deduplication: HashMap<String, (u64, Original)>,
}

impl Queue {
    pub fn new(id: QueueId, config: QueueConfig) -> Self {
        Self {
            id,
            config,
            messages: BTreeMap::new(),
            next_sequence: 1,
            deduplication: HashMap::new(),
        }
    }

    pub fn id(&self) -> QueueId {
        self.id
    }

    pub fn config(&self) -> &QueueConfig {
        &self.config
    }

    pub fn len(&self) -> usize {
        self.messages.len()
    }

    pub fn set_attributes(
        &mut self,
        update: &AttributeUpdate,
        dead_letter: Option<DeadLetter>,
        now: u64,
    ) {
        self.config.apply(update, dead_letter);
        if update.retention_seconds.is_some() {
            self.sweep(now);
        }
    }

    pub fn clear_dead_letter(&mut self) {
        self.config.dead_letter = None;
    }

    pub fn counts(&self, now: u64) -> Counts {
        let mut counts = Counts {
            visible: 0,
            in_flight: 0,
            delayed: 0,
        };
        for message in self.messages.values() {
            match message.state(now) {
                State::Visible => counts.visible += 1,
                State::InFlight => counts.in_flight += 1,
                State::Delayed => counts.delayed += 1,
            }
        }
        counts
    }

    /// Removes every message, keeping sequence numbers and deduplication ids.
    pub fn purge(&mut self) {
        self.messages.clear();
    }

    /// Drops messages past retention and deduplication ids whose window has closed.
    ///
    /// Retention is only enforced by this call and on dead-letter arrival: a message
    /// sent at `now` can never be expired yet, so nothing else can create an expired one.
    pub fn sweep(&mut self, now: u64) {
        let retention = self.config.retention_seconds;
        self.messages.retain(|_, m| now < m.sent_at + retention);
        self.deduplication
            .retain(|_, (at, _)| now < *at + DEDUPLICATION_WINDOW);
    }

    pub fn duplicate_of(&self, deduplication_id: &str, now: u64) -> Option<Original> {
        let (at, original) = self.deduplication.get(deduplication_id)?;
        (now < at + DEDUPLICATION_WINDOW).then_some(*original)
    }

    /// Adds a message at the end of the queue and returns its sequence number.
    pub fn accept(&mut self, new: NewMessage, now: u64) -> u64 {
        let sequence = self.take_sequence();
        let (group_id, deduplication_id) = match new.fifo {
            Some(fifo) => (Some(fifo.group_id), Some(fifo.deduplication_id)),
            None => (None, None),
        };
        if let Some(deduplication_id) = deduplication_id {
            let original = Original {
                id: new.id,
                sequence,
            };
            self.deduplication.insert(deduplication_id, (now, original));
        }
        let hold = (new.delay_seconds > 0).then_some(Hold::Delayed {
            until: now + new.delay_seconds,
        });
        self.messages.insert(
            sequence,
            Message {
                id: new.id,
                body: new.body,
                sent_at: now,
                receive_count: 0,
                first_received_at: None,
                group_id,
                hold,
            },
        );
        sequence
    }

    /// Adds a message moved from another queue: fresh counters, visible at once, and
    /// dropped on arrival when it is already past this queue's retention.
    pub fn accept_dead_letter(&mut self, mut message: Message, now: u64) {
        let sequence = self.take_sequence();
        message.receive_count = 0;
        message.first_received_at = None;
        message.hold = None;
        if now < message.sent_at + self.config.retention_seconds {
            self.messages.insert(sequence, message);
        }
    }

    fn take_sequence(&mut self) -> u64 {
        let sequence = self.next_sequence;
        self.next_sequence += 1;
        sequence
    }

    /// Delivers up to `max_messages` messages, moving exhausted ones out of the queue.
    pub fn receive(
        &mut self,
        now: u64,
        max_messages: u64,
        visibility_timeout: Option<u64>,
        log: &mut DeliveryLog,
    ) -> Received {
        let visibility_timeout = visibility_timeout.unwrap_or(self.config.visibility_timeout);
        let mut received = Received {
            delivered: Vec::new(),
            dead: Vec::new(),
        };
        for action in self.plan_receive(now, max_messages) {
            match action {
                Action::DeadLetter(sequence) => {
                    received.dead.extend(self.messages.remove(&sequence));
                }
                Action::Deliver(sequence) => {
                    let delivered = self.deliver(sequence, now, visibility_timeout, log);
                    received.delivered.push(delivered);
                }
            }
        }
        received
    }

    /// Examines the messages in arrival order and decides what happens to each one.
    ///
    /// A message that stays in the queue, in any state, blocks later messages of its
    /// group. One that is dead-lettered has left the queue and does not.
    fn plan_receive(&self, now: u64, max_messages: u64) -> Vec<Action> {
        let mut actions = Vec::new();
        let mut delivered = 0;
        let mut blocked_groups: HashSet<&str> = HashSet::new();
        for (&sequence, message) in &self.messages {
            if delivered == max_messages {
                break;
            }
            let group = message.group_id.as_deref();
            let blocked = group.is_some_and(|group| blocked_groups.contains(group));
            if message.state(now) == State::Visible && !blocked {
                if self.is_exhausted(message) {
                    actions.push(Action::DeadLetter(sequence));
                    continue;
                }
                actions.push(Action::Deliver(sequence));
                delivered += 1;
            }
            blocked_groups.extend(group);
        }
        actions
    }

    fn is_exhausted(&self, message: &Message) -> bool {
        self.config
            .dead_letter
            .as_ref()
            .is_some_and(|dead_letter| message.receive_count >= dead_letter.max_receive_count)
    }

    fn deliver(
        &mut self,
        sequence: u64,
        now: u64,
        visibility_timeout: u64,
        log: &mut DeliveryLog,
    ) -> Delivered {
        let message = self
            .messages
            .get_mut(&sequence)
            .expect("receive plans only sequence numbers of messages in the queue");
        let handle = log.issue(self.id, sequence);
        message.receive_count += 1;
        let first_received_at = *message.first_received_at.get_or_insert(now);
        message.hold = Some(Hold::InFlight {
            until: now + visibility_timeout,
            handle,
        });
        Delivered {
            message_id: message.id,
            receipt_handle: handle,
            body: message.body.clone(),
            receive_count: message.receive_count,
            sent_at: message.sent_at,
            first_received_at,
            group_id: message.group_id.clone(),
            sequence_number: self.config.fifo.then_some(sequence),
        }
    }

    /// Whether `handle` is the latest delivery of the message with this sequence number
    /// and that message is in flight.
    pub fn is_in_flight(&self, sequence: u64, handle: ReceiptHandle, now: u64) -> bool {
        self.messages
            .get(&sequence)
            .and_then(|message| message.in_flight_handle(now))
            == Some(handle)
    }

    pub fn delete(&mut self, sequence: u64) {
        self.messages.remove(&sequence);
    }

    /// Makes the in-flight message `sequence` hidden until `now + timeout`.
    pub fn change_visibility(&mut self, sequence: u64, now: u64, timeout: u64) {
        let message = self
            .messages
            .get_mut(&sequence)
            .expect("callers check is_in_flight first");
        if let Some(Hold::InFlight { until, .. }) = &mut message.hold {
            *until = now + timeout;
        }
    }
}
