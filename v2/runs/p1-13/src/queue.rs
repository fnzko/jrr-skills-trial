//! One queue: its settings, its messages and the indexes that keep every
//! operation cheap.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use serde::Serialize;

use crate::ids::{MessageId, ReceiptHandle, SequenceNumber};
use crate::scalar::{
    Counter, DelaySeconds, MaxMessages, MaxReceiveCount, RetentionSeconds, Timestamp,
    VisibilityTimeout,
};
use crate::text::{Body, DedupKey, GroupId, QueueName};

/// How long a deduplication id stays in force after it was accepted.
const DEDUP_WINDOW_SECONDS: u32 = 300;

/// Most messages a queue may hold, in any state.
pub const MAX_MESSAGES: usize = 100_000;

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

/// The attributes that always have a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueueSettings {
    pub visibility_timeout: VisibilityTimeout,
    pub delay: DelaySeconds,
    pub retention: RetentionSeconds,
    pub kind: QueueKind,
}

impl QueueSettings {
    pub fn with_defaults(kind: QueueKind) -> Self {
        Self {
            visibility_timeout: VisibilityTimeout::literal::<30>(),
            delay: DelaySeconds::literal::<0>(),
            retention: RetentionSeconds::literal::<345_600>(),
            kind,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeadLetterConfig {
    pub queue: QueueName,
    pub max_receive_count: MaxReceiveCount,
}

/// Where a message of a FIFO queue sits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FifoPlacement {
    #[serde(rename = "GroupId")]
    pub group: GroupId,
    #[serde(rename = "SequenceNumber")]
    pub sequence: SequenceNumber,
}

/// What a FIFO send adds to the plain message.
pub struct FifoSend {
    pub group: GroupId,
    pub dedup: DedupKey,
}

/// A send remembered for deduplication.
#[derive(Debug, Clone, Copy)]
pub struct Accepted {
    pub id: MessageId,
    pub sequence: SequenceNumber,
    at: Timestamp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Availability {
    Visible,
    Delayed { until: Timestamp },
    InFlight { until: Timestamp },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Visible,
    InFlight,
    Delayed,
}

#[derive(Debug)]
struct Message {
    id: MessageId,
    body: Body,
    sent_at: Timestamp,
    receive_count: u32,
    first_received_at: Option<Timestamp>,
    fifo: Option<FifoPlacement>,
    availability: Availability,
    /// The latest handle issued for this message; also its key in `Queue::handles`.
    handle: Option<ReceiptHandle>,
}

impl Message {
    fn state(&self, now: Timestamp) -> State {
        match self.availability {
            Availability::Delayed { until } if now < until => State::Delayed,
            Availability::InFlight { until } if now < until => State::InFlight,
            Availability::Visible
            | Availability::Delayed { .. }
            | Availability::InFlight { .. } => State::Visible,
        }
    }

    fn group(&self) -> Option<&GroupId> {
        self.fifo.as_ref().map(|placement| &placement.group)
    }
}

/// Position of a message in arrival order within its queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Arrival(u64);

/// A message handed to a consumer.
#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Delivered {
    pub message_id: MessageId,
    pub receipt_handle: ReceiptHandle,
    pub body: Body,
    pub receive_count: u32,
    pub sent_at: Timestamp,
    pub first_received_at: Timestamp,
    #[serde(flatten)]
    pub fifo: Option<FifoPlacement>,
}

/// A message on its way to a dead-letter queue.
#[derive(Debug)]
pub struct DeadLetter {
    id: MessageId,
    body: Body,
    sent_at: Timestamp,
    group: Option<GroupId>,
}

#[derive(Debug, Default)]
pub struct Receipt {
    pub delivered: Vec<Delivered>,
    pub dead: Vec<DeadLetter>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Counts {
    pub visible: usize,
    pub in_flight: usize,
    pub delayed: usize,
}

#[derive(Debug, Clone, Copy)]
enum Step {
    Deliver,
    DeadLetter,
}

#[derive(Debug)]
pub struct Queue {
    settings: QueueSettings,
    dead_letter: Option<DeadLetterConfig>,
    messages: BTreeMap<Arrival, Message>,
    /// Messages by send time, so retention only looks at the oldest ones.
    by_age: BTreeSet<(Timestamp, Arrival)>,
    handles: HashMap<ReceiptHandle, Arrival>,
    /// Never pruned: ids outlive their messages.
    dedup: HashMap<DedupKey, Accepted>,
    next_arrival: u64,
    sequences: Counter,
}

impl Queue {
    pub fn new(settings: QueueSettings, dead_letter: Option<DeadLetterConfig>) -> Self {
        Self {
            settings,
            dead_letter,
            messages: BTreeMap::new(),
            by_age: BTreeSet::new(),
            handles: HashMap::new(),
            dedup: HashMap::new(),
            next_arrival: 0,
            sequences: Counter::default(),
        }
    }

    pub fn settings(&self) -> &QueueSettings {
        &self.settings
    }

    pub fn dead_letter(&self) -> Option<&DeadLetterConfig> {
        self.dead_letter.as_ref()
    }

    pub fn clear_dead_letter(&mut self) {
        self.dead_letter = None;
    }

    /// Replaces the settings and, when given, the dead-letter configuration.
    /// Retention applies to the messages at once.
    pub fn update(
        &mut self,
        settings: QueueSettings,
        dead_letter: Option<DeadLetterConfig>,
        now: Timestamp,
    ) {
        self.settings = settings;
        if dead_letter.is_some() {
            self.dead_letter = dead_letter;
        }
        self.expire(now);
    }

    pub fn is_full(&self) -> bool {
        self.messages.len() >= MAX_MESSAGES
    }

    /// Removes every message that is past its retention. Must run before any
    /// command looks at or changes the queue.
    pub fn expire(&mut self, now: Timestamp) {
        let retention = self.settings.retention.get();
        while let Some(&(sent_at, arrival)) = self.by_age.first() {
            if now < sent_at.plus(retention) {
                break;
            }
            self.remove(arrival);
        }
    }

    pub fn counts(&self, now: Timestamp) -> Counts {
        self.messages
            .values()
            .fold(Counts::default(), |mut counts, message| {
                match message.state(now) {
                    State::Visible => counts.visible += 1,
                    State::InFlight => counts.in_flight += 1,
                    State::Delayed => counts.delayed += 1,
                }
                counts
            })
    }

    pub fn purge(&mut self) {
        self.messages.clear();
        self.by_age.clear();
        self.handles.clear();
    }

    /// The send that a new send with this deduplication id would repeat.
    pub fn find_duplicate(&self, dedup: &str, now: Timestamp) -> Option<&Accepted> {
        self.dedup
            .get(dedup)
            .filter(|accepted| now < accepted.at.plus(DEDUP_WINDOW_SECONDS))
    }

    /// Adds a new message and returns its sequence number in a FIFO queue.
    pub fn accept(
        &mut self,
        id: MessageId,
        body: Body,
        now: Timestamp,
        delay: DelaySeconds,
        fifo: Option<FifoSend>,
    ) -> Option<SequenceNumber> {
        let placement = fifo.map(|send| {
            let sequence = SequenceNumber::new(self.sequences.issue());
            self.dedup.insert(
                send.dedup,
                Accepted {
                    id,
                    sequence,
                    at: now,
                },
            );
            FifoPlacement {
                group: send.group,
                sequence,
            }
        });
        let sequence = placement.as_ref().map(|placement| placement.sequence);
        self.insert(Message {
            id,
            body,
            sent_at: now,
            receive_count: 0,
            first_received_at: None,
            fifo: placement,
            availability: Availability::Delayed {
                until: now.plus(delay.get()),
            },
            handle: None,
        });
        sequence
    }

    /// Adds a message moved from another queue, which keeps counting its
    /// retention from the original send time.
    pub fn accept_dead_letter(&mut self, dead: DeadLetter, now: Timestamp) {
        let fifo = dead.group.map(|group| FifoPlacement {
            group,
            sequence: SequenceNumber::new(self.sequences.issue()),
        });
        self.insert(Message {
            id: dead.id,
            body: dead.body,
            sent_at: dead.sent_at,
            receive_count: 0,
            first_received_at: None,
            fifo,
            availability: Availability::Visible,
            handle: None,
        });
        self.expire(now);
    }

    /// Delivers up to `max` eligible messages, moving out those that have
    /// reached the dead-letter limit.
    pub fn receive(
        &mut self,
        now: Timestamp,
        max: MaxMessages,
        visibility_timeout: VisibilityTimeout,
        handles: &mut Counter,
    ) -> Receipt {
        let mut receipt = Receipt::default();
        for (arrival, step) in self.plan_receive(now, max) {
            match step {
                Step::Deliver => {
                    let message = self
                        .messages
                        .get_mut(&arrival)
                        .expect("a planned arrival is still in the queue");
                    let handle = ReceiptHandle::new(handles.issue());
                    if let Some(old) = message.handle.replace(handle) {
                        self.handles.remove(&old);
                    }
                    self.handles.insert(handle, arrival);
                    message.receive_count += 1;
                    let first_received_at = *message.first_received_at.get_or_insert(now);
                    message.availability = Availability::InFlight {
                        until: now.plus(visibility_timeout.get()),
                    };
                    receipt.delivered.push(Delivered {
                        message_id: message.id,
                        receipt_handle: handle,
                        body: message.body.clone(),
                        receive_count: message.receive_count,
                        sent_at: message.sent_at,
                        first_received_at,
                        fifo: message.fifo.clone(),
                    });
                }
                Step::DeadLetter => {
                    let message = self
                        .remove(arrival)
                        .expect("a planned arrival is still in the queue");
                    receipt.dead.push(DeadLetter {
                        id: message.id,
                        body: message.body,
                        sent_at: message.sent_at,
                        group: message.fifo.map(|placement| placement.group),
                    });
                }
            }
        }
        receipt
    }

    /// Examines messages in arrival order and decides what happens to each.
    /// A message that stays in the queue holds back the later ones of its
    /// group, whether or not it was delivered.
    fn plan_receive(&self, now: Timestamp, max: MaxMessages) -> Vec<(Arrival, Step)> {
        let dead_after = self
            .dead_letter
            .as_ref()
            .map(|config| config.max_receive_count.get());
        let mut held_groups: HashSet<&GroupId> = HashSet::new();
        let mut plan = Vec::new();
        let mut delivered = 0;
        for (&arrival, message) in &self.messages {
            let eligible = message.state(now) == State::Visible
                && message
                    .group()
                    .is_none_or(|group| !held_groups.contains(group));
            if eligible && dead_after.is_some_and(|limit| message.receive_count >= limit) {
                plan.push((arrival, Step::DeadLetter));
                continue;
            }
            if let Some(group) = message.group() {
                held_groups.insert(group);
            }
            if eligible {
                plan.push((arrival, Step::Deliver));
                delivered += 1;
                if delivered == max.get() {
                    break;
                }
            }
        }
        plan
    }

    /// The message whose latest handle this is, if it is in flight.
    pub fn find_in_flight(&self, handle: ReceiptHandle, now: Timestamp) -> Option<Arrival> {
        let arrival = *self.handles.get(&handle)?;
        let message = self.messages.get(&arrival)?;
        (message.state(now) == State::InFlight).then_some(arrival)
    }

    pub fn delete(&mut self, arrival: Arrival) {
        self.remove(arrival);
    }

    /// Makes the message in flight until `now + timeout`.
    pub fn change_visibility(
        &mut self,
        arrival: Arrival,
        now: Timestamp,
        timeout: VisibilityTimeout,
    ) {
        let message = self
            .messages
            .get_mut(&arrival)
            .expect("the arrival was returned by find_in_flight on this queue");
        message.availability = Availability::InFlight {
            until: now.plus(timeout.get()),
        };
    }

    fn insert(&mut self, message: Message) {
        let arrival = Arrival(self.next_arrival);
        self.next_arrival += 1;
        self.by_age.insert((message.sent_at, arrival));
        self.messages.insert(arrival, message);
    }

    fn remove(&mut self, arrival: Arrival) -> Option<Message> {
        let message = self.messages.remove(&arrival)?;
        self.by_age.remove(&(message.sent_at, arrival));
        if let Some(handle) = message.handle {
            self.handles.remove(&handle);
        }
        Some(message)
    }
}
