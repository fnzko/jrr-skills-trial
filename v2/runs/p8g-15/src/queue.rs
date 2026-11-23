//! One queue: its configuration and its messages, with the state rules of the spec.

use std::collections::{BTreeMap, HashMap, HashSet};

use serde::Serialize;

use crate::types::{
    Body, DelaySeconds, GroupId, IdAllocator, MaxMessages, MaxReceiveCount, MessageId, QueueName,
    ReceiptHandle, RetentionSeconds, SequenceNumber, Timestamp, VisibilityTimeout,
};

pub const MAX_MESSAGES_PER_QUEUE: usize = 100_000;
const DEDUPLICATION_WINDOW_SECONDS: u32 = 300;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueKind {
    Standard,
    Fifo { content_based_deduplication: bool },
}

impl QueueKind {
    pub fn is_fifo(self) -> bool {
        matches!(self, Self::Fifo { .. })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeadLetterConfig {
    pub queue: QueueName,
    pub max_receive_count: MaxReceiveCount,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueConfig {
    pub visibility_timeout: VisibilityTimeout,
    pub delay: DelaySeconds,
    pub retention: RetentionSeconds,
    pub kind: QueueKind,
    pub dead_letter: Option<DeadLetterConfig>,
}

/// How many messages are in each state.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Counts {
    pub visible: usize,
    pub in_flight: usize,
    pub delayed: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct FifoInfo {
    pub group_id: GroupId,
    pub sequence_number: SequenceNumber,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Delayed,
    InFlight,
    Visible,
}

#[derive(Debug, Clone)]
struct Delivery {
    handle: ReceiptHandle,
    visible_at: Timestamp,
}

/// A message in a queue.
#[derive(Debug, Clone)]
pub struct Message {
    id: MessageId,
    body: Body,
    sent_at: Timestamp,
    delayed_until: Timestamp,
    receive_count: u32,
    first_received_at: Option<Timestamp>,
    delivery: Option<Delivery>,
    fifo: Option<FifoInfo>,
}

impl Message {
    fn state(&self, now: Timestamp) -> State {
        if now < self.delayed_until {
            State::Delayed
        } else if self.delivery.as_ref().is_some_and(|d| now < d.visible_at) {
            State::InFlight
        } else {
            State::Visible
        }
    }
}

/// A delivered message as `ReceiveMessage` reports it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct DeliveredMessage {
    pub message_id: MessageId,
    pub receipt_handle: ReceiptHandle,
    pub body: Body,
    pub receive_count: u32,
    pub sent_at: Timestamp,
    pub first_received_at: Timestamp,
    #[serde(flatten)]
    pub fifo: Option<FifoInfo>,
}

/// The outcome of one `ReceiveMessage` on a queue.
#[derive(Debug, Default)]
pub struct Received {
    pub delivered: Vec<DeliveredMessage>,
    /// Messages to append to the queue named by the dead-letter configuration.
    pub dead_lettered: Vec<Message>,
}

/// What the broker hands over for a message that passed the send checks.
pub struct Submission {
    pub id: MessageId,
    pub body: Body,
    pub now: Timestamp,
    pub delay: DelaySeconds,
    pub fifo: Option<FifoSubmission>,
}

pub struct FifoSubmission {
    pub group: GroupId,
    /// The deduplication id: the caller's `DeduplicationId`, else the body.
    pub deduplication_key: String,
}

#[derive(Debug)]
struct DeduplicationRecord {
    accepted_at: Timestamp,
    id: MessageId,
    sequence: SequenceNumber,
}

/// Position in arrival order; never reused within a queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Arrival(u64);

#[derive(Clone, Copy)]
enum Action {
    Deliver,
    DeadLetter,
}

#[derive(Debug)]
pub struct Queue {
    config: QueueConfig,
    messages: BTreeMap<Arrival, Message>,
    next_arrival: Arrival,
    next_sequence: SequenceNumber,
    /// The latest handle of each message that has been delivered, for lookup by handle.
    handles: HashMap<ReceiptHandle, Arrival>,
    deduplication: HashMap<String, DeduplicationRecord>,
    /// A lower bound of the `sent_at` of every message, so that retention checks are
    /// constant time unless something may have expired.
    oldest_sent: Option<Timestamp>,
}

impl Queue {
    pub fn new(config: QueueConfig) -> Self {
        Self {
            config,
            messages: BTreeMap::new(),
            next_arrival: Arrival(0),
            next_sequence: SequenceNumber::FIRST,
            handles: HashMap::new(),
            deduplication: HashMap::new(),
            oldest_sent: None,
        }
    }

    pub fn config(&self) -> &QueueConfig {
        &self.config
    }

    pub fn is_full(&self) -> bool {
        self.messages.len() >= MAX_MESSAGES_PER_QUEUE
    }

    /// Removes every message whose retention has run out. Callers do this before they
    /// look at the queue, which makes retention hold at every moment.
    pub fn expire(&mut self, now: Timestamp) {
        let retention = self.config.retention.get();
        match self.oldest_sent {
            Some(oldest) if now >= oldest.after(retention) => {}
            _ => return,
        }
        self.messages
            .retain(|_, message| now < message.sent_at.after(retention));
        self.oldest_sent = self.messages.values().map(|m| m.sent_at).min();
        let messages = &self.messages;
        self.handles
            .retain(|_, arrival| messages.contains_key(arrival));
    }

    /// Replaces the configuration. Messages that the old retention had already
    /// expired are removed first, so a longer retention cannot bring them back.
    pub fn reconfigure(&mut self, now: Timestamp, config: QueueConfig) {
        self.expire(now);
        self.config = config;
        self.expire(now);
    }

    pub fn drop_dead_letter_to(&mut self, deleted: &QueueName) {
        if self
            .config
            .dead_letter
            .as_ref()
            .is_some_and(|dead_letter| dead_letter.queue == *deleted)
        {
            self.config.dead_letter = None;
        }
    }

    pub fn counts(&self, now: Timestamp) -> Counts {
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

    /// Removes every message. Sequence numbers and deduplication records carry on.
    pub fn purge(&mut self) {
        self.messages.clear();
        self.handles.clear();
        self.oldest_sent = None;
    }

    /// The message a FIFO send with this deduplication id duplicates, if any.
    pub fn find_duplicate(
        &self,
        deduplication_key: &str,
        now: Timestamp,
    ) -> Option<(MessageId, SequenceNumber)> {
        let record = self.deduplication.get(deduplication_key)?;
        let window_end = record.accepted_at.after(DEDUPLICATION_WINDOW_SECONDS);
        (now < window_end).then_some((record.id, record.sequence))
    }

    /// Appends the message. The result is its sequence number on a FIFO queue.
    pub fn accept(&mut self, submission: Submission) -> Option<SequenceNumber> {
        let Submission {
            id,
            body,
            now,
            delay,
            fifo,
        } = submission;
        let fifo = fifo.map(|fifo| {
            let sequence = self.take_sequence();
            self.deduplication.insert(
                fifo.deduplication_key,
                DeduplicationRecord {
                    accepted_at: now,
                    id,
                    sequence,
                },
            );
            FifoInfo {
                group_id: fifo.group,
                sequence_number: sequence,
            }
        });
        let sequence = fifo.as_ref().map(|info| info.sequence_number);
        self.append(Message {
            id,
            body,
            sent_at: now,
            delayed_until: now.after(delay.get()),
            receive_count: 0,
            first_received_at: None,
            delivery: None,
            fifo,
        });
        sequence
    }

    /// Appends a message that moved here from another queue, as the spec describes,
    /// and drops it again at once if its retention here has already run out.
    pub fn accept_dead_letter(&mut self, mut message: Message, now: Timestamp) {
        message.receive_count = 0;
        message.first_received_at = None;
        message.delivery = None;
        message.delayed_until = message.sent_at;
        if message.fifo.is_some() {
            let sequence = self.take_sequence();
            if let Some(info) = &mut message.fifo {
                info.sequence_number = sequence;
            }
        }
        self.append(message);
        self.expire(now);
    }

    /// Delivers up to `max` messages and picks out those that go to the dead-letter queue.
    pub fn receive(
        &mut self,
        now: Timestamp,
        max: MaxMessages,
        visibility: VisibilityTimeout,
        ids: &mut IdAllocator,
    ) -> Received {
        let mut received = Received::default();
        for (arrival, action) in self.plan_receive(now, max) {
            match action {
                Action::Deliver => {
                    let handle = ids.next_handle();
                    if let Some(delivered) = self.deliver(arrival, handle, now, visibility) {
                        received.delivered.push(delivered);
                    }
                }
                Action::DeadLetter => {
                    if let Some(message) = self.remove(arrival) {
                        received.dead_lettered.push(message);
                    }
                }
            }
        }
        received
    }

    /// Fails with `None` unless `handle` is the latest handle of a message in flight now.
    pub fn delete_in_flight(&mut self, handle: ReceiptHandle, now: Timestamp) -> Option<()> {
        let arrival = self.in_flight(handle, now)?;
        self.remove(arrival).map(|_| ())
    }

    /// Fails with `None` unless `handle` is the latest handle of a message in flight now.
    pub fn change_visibility(
        &mut self,
        handle: ReceiptHandle,
        now: Timestamp,
        timeout: VisibilityTimeout,
    ) -> Option<()> {
        let arrival = self.in_flight(handle, now)?;
        let delivery = self.messages.get_mut(&arrival)?.delivery.as_mut()?;
        delivery.visible_at = now.after(timeout.get());
        Some(())
    }

    fn in_flight(&self, handle: ReceiptHandle, now: Timestamp) -> Option<Arrival> {
        let arrival = *self.handles.get(&handle)?;
        let message = self.messages.get(&arrival)?;
        (message.state(now) == State::InFlight).then_some(arrival)
    }

    fn take_sequence(&mut self) -> SequenceNumber {
        let sequence = self.next_sequence;
        self.next_sequence = sequence.next();
        sequence
    }

    fn append(&mut self, message: Message) {
        let arrival = self.next_arrival;
        self.next_arrival = Arrival(arrival.0 + 1);
        self.oldest_sent = Some(
            self.oldest_sent
                .map_or(message.sent_at, |oldest| oldest.min(message.sent_at)),
        );
        self.messages.insert(arrival, message);
    }

    fn remove(&mut self, arrival: Arrival) -> Option<Message> {
        let message = self.messages.remove(&arrival)?;
        if let Some(delivery) = &message.delivery {
            self.handles.remove(&delivery.handle);
        }
        Some(message)
    }

    /// Decides, without changing anything, what one receive does to which message.
    fn plan_receive(&self, now: Timestamp, max: MaxMessages) -> Vec<(Arrival, Action)> {
        let limit = self
            .config
            .dead_letter
            .as_ref()
            .map(|dead_letter| dead_letter.max_receive_count.get());
        // Groups that already have an earlier message in the queue.
        let mut occupied: HashSet<&GroupId> = HashSet::new();
        let mut plan = Vec::new();
        let mut deliveries = 0;
        for (&arrival, message) in &self.messages {
            if deliveries == max.get() {
                break;
            }
            let group = message.fifo.as_ref().map(|info| &info.group_id);
            if group.is_some_and(|group| occupied.contains(group)) {
                continue;
            }
            if message.state(now) == State::Visible {
                if limit.is_some_and(|limit| message.receive_count >= limit) {
                    // It leaves the queue, so it does not hold its group back.
                    plan.push((arrival, Action::DeadLetter));
                    continue;
                }
                plan.push((arrival, Action::Deliver));
                deliveries += 1;
            }
            occupied.extend(group);
        }
        plan
    }

    fn deliver(
        &mut self,
        arrival: Arrival,
        handle: ReceiptHandle,
        now: Timestamp,
        visibility: VisibilityTimeout,
    ) -> Option<DeliveredMessage> {
        let message = self.messages.get_mut(&arrival)?;
        if let Some(previous) = &message.delivery {
            self.handles.remove(&previous.handle);
        }
        message.receive_count += 1;
        let first_received_at = *message.first_received_at.get_or_insert(now);
        message.delivery = Some(Delivery {
            handle,
            visible_at: now.after(visibility.get()),
        });
        self.handles.insert(handle, arrival);
        Some(DeliveredMessage {
            message_id: message.id,
            receipt_handle: handle,
            body: message.body.clone(),
            receive_count: message.receive_count,
            sent_at: message.sent_at,
            first_received_at,
            fifo: message.fifo.clone(),
        })
    }
}
