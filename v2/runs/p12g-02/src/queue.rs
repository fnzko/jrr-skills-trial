//! One queue: its attributes, its messages in arrival order and the per-message state machine.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use serde::Serialize;

use crate::request::AttributeUpdate;
use crate::response::ReceivedMessage;
use crate::types::{
    DeduplicationId, GroupId, Ids, Instant, MessageId, QueueName, ReceiptHandle, Seconds,
    SequenceNumber,
};

pub const DEFAULT_VISIBILITY_TIMEOUT: Seconds = Seconds(30);
pub const DEFAULT_RETENTION: Seconds = Seconds(345_600);
pub const MAX_MESSAGES_PER_QUEUE: usize = 100_000;

/// How long a FIFO queue remembers a deduplication id after accepting a send with it.
const DEDUPLICATION_WINDOW: Seconds = Seconds(300);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueKind {
    Standard,
    Fifo { content_based_deduplication: bool },
}

impl QueueKind {
    pub fn is_fifo(self) -> bool {
        matches!(self, QueueKind::Fifo { .. })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct DeadLetterConfig {
    #[serde(rename = "DeadLetterQueue")]
    pub queue: QueueName,
    pub max_receive_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueConfig {
    pub visibility_timeout: Seconds,
    pub delay: Seconds,
    pub retention: Seconds,
    pub kind: QueueKind,
    pub dead_letter: Option<DeadLetterConfig>,
}

impl QueueConfig {
    pub fn new(kind: QueueKind) -> Self {
        Self {
            visibility_timeout: DEFAULT_VISIBILITY_TIMEOUT,
            delay: Seconds(0),
            retention: DEFAULT_RETENTION,
            kind,
            dead_letter: None,
        }
    }

    /// Takes the timing attributes that `update` gives and keeps the others.
    pub fn apply_timings(&mut self, update: &AttributeUpdate) {
        self.visibility_timeout = update.visibility_timeout.unwrap_or(self.visibility_timeout);
        self.delay = update.delay.unwrap_or(self.delay);
        self.retention = update.retention.unwrap_or(self.retention);
    }
}

/// Where a FIFO message sits: its group and its number in the queue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct FifoPlacement {
    pub group_id: GroupId,
    pub sequence_number: SequenceNumber,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MessageState {
    Delayed,
    InFlight,
    Visible,
}

#[derive(Debug)]
struct Delivery {
    receipt: ReceiptHandle,
    invisible_until: Instant,
}

#[derive(Debug)]
struct Message {
    id: MessageId,
    body: String,
    sent_at: Instant,
    receive_count: u32,
    first_received_at: Option<Instant>,
    fifo: Option<FifoPlacement>,
    delayed_until: Option<Instant>,
    delivery: Option<Delivery>,
}

impl Message {
    fn state(&self, now: Instant) -> MessageState {
        if self.delayed_until.is_some_and(|until| now < until) {
            MessageState::Delayed
        } else if self
            .delivery
            .as_ref()
            .is_some_and(|delivery| now < delivery.invisible_until)
        {
            MessageState::InFlight
        } else {
            MessageState::Visible
        }
    }
}

/// A message about to be appended to a queue, by a send or a dead-letter move.
#[derive(Debug)]
pub struct NewMessage {
    pub id: MessageId,
    pub body: String,
    pub sent_at: Instant,
    pub group: Option<GroupId>,
    pub delayed_until: Option<Instant>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Counts {
    pub visible: usize,
    pub in_flight: usize,
    pub delayed: usize,
}

/// The first accepted send of a deduplication id in the current window.
#[derive(Debug, Clone, Copy)]
pub struct Accepted {
    pub message_id: MessageId,
    pub sequence: SequenceNumber,
    at: Instant,
}

/// The result of a `ReceiveMessage` on one queue.
#[derive(Debug, Default)]
pub struct Received {
    pub delivered: Vec<ReceivedMessage>,
    /// Messages removed from this queue because they reached the maximum receive count, in
    /// the order they must be appended to the dead-letter queue.
    pub dead_lettered: Vec<NewMessage>,
}

/// The handle does not name a message that is in flight in this queue.
#[derive(Debug, PartialEq, Eq)]
pub struct NotInFlight;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Arrival(u64);

enum Fate {
    Deliver,
    DeadLetter,
}

#[derive(Debug)]
pub struct Queue {
    config: QueueConfig,
    messages: BTreeMap<Arrival, Message>,
    last_arrival: u64,
    /// Messages by send time, so retention needs no scan of the whole queue.
    by_age: BTreeSet<(Instant, Arrival)>,
    /// Receipt handles ever issued by this queue. A handle is usable only if it is also the
    /// latest handle of a message that is in flight.
    receipts: HashMap<ReceiptHandle, Arrival>,
    deduplication: HashMap<DeduplicationId, Accepted>,
    last_sequence: u64,
}

impl Queue {
    pub fn new(config: QueueConfig) -> Self {
        Self {
            config,
            messages: BTreeMap::new(),
            last_arrival: 0,
            by_age: BTreeSet::new(),
            receipts: HashMap::new(),
            deduplication: HashMap::new(),
            last_sequence: 0,
        }
    }

    pub fn config(&self) -> &QueueConfig {
        &self.config
    }

    pub fn config_mut(&mut self) -> &mut QueueConfig {
        &mut self.config
    }

    pub fn len(&self) -> usize {
        self.messages.len()
    }

    /// Removes the messages whose retention has run out at `now`. Callers do this before every
    /// read or change, and after a retention change.
    pub fn expire(&mut self, now: Instant) {
        let retention = self.config.retention;
        while let Some(&(sent_at, arrival)) = self.by_age.first() {
            if sent_at.plus(retention) > now {
                break;
            }
            self.remove(arrival);
        }
    }

    fn remove(&mut self, arrival: Arrival) -> Option<Message> {
        let message = self.messages.remove(&arrival)?;
        self.by_age.remove(&(message.sent_at, arrival));
        Some(message)
    }

    pub fn counts(&self, now: Instant) -> Counts {
        let mut counts = Counts::default();
        for message in self.messages.values() {
            match message.state(now) {
                MessageState::Visible => counts.visible += 1,
                MessageState::InFlight => counts.in_flight += 1,
                MessageState::Delayed => counts.delayed += 1,
            }
        }
        counts
    }

    /// Appends a message and returns its sequence number if this queue is FIFO. The caller
    /// passes a group exactly when the queue is FIFO.
    pub fn add(&mut self, new: NewMessage) -> Option<SequenceNumber> {
        let fifo = new.group.map(|group_id| {
            self.last_sequence += 1;
            FifoPlacement {
                group_id,
                sequence_number: SequenceNumber(self.last_sequence),
            }
        });
        let sequence = fifo.as_ref().map(|placement| placement.sequence_number);
        self.last_arrival += 1;
        let arrival = Arrival(self.last_arrival);
        self.by_age.insert((new.sent_at, arrival));
        self.messages.insert(
            arrival,
            Message {
                id: new.id,
                body: new.body,
                sent_at: new.sent_at,
                receive_count: 0,
                first_received_at: None,
                fifo,
                delayed_until: new.delayed_until,
                delivery: None,
            },
        );
        sequence
    }

    /// The original send, if `id` was accepted less than the deduplication window ago.
    pub fn duplicate_of(&self, id: &DeduplicationId, now: Instant) -> Option<Accepted> {
        self.deduplication
            .get(id)
            .filter(|accepted| now < accepted.at.plus(DEDUPLICATION_WINDOW))
            .copied()
    }

    /// Opens a new deduplication window for `id`.
    pub fn record_accepted(
        &mut self,
        id: DeduplicationId,
        message_id: MessageId,
        sequence: SequenceNumber,
        now: Instant,
    ) {
        self.deduplication.insert(
            id,
            Accepted {
                message_id,
                sequence,
                at: now,
            },
        );
    }

    /// Removes every message. Sequence numbers and deduplication carry on.
    pub fn purge(&mut self) {
        self.messages.clear();
        self.by_age.clear();
        self.receipts.clear();
    }

    pub fn receive(
        &mut self,
        now: Instant,
        max_messages: u32,
        visibility_timeout: Seconds,
        ids: &mut Ids,
    ) -> Received {
        let mut received = Received::default();
        for (arrival, fate) in self.plan_receive(now, max_messages) {
            match fate {
                Fate::Deliver => {
                    let receipt = ids.next_receipt();
                    let message = self
                        .messages
                        .get_mut(&arrival)
                        .expect("a planned arrival is still queued");
                    message.receive_count += 1;
                    let first_received_at = *message.first_received_at.get_or_insert(now);
                    message.delivery = Some(Delivery {
                        receipt,
                        invisible_until: now.plus(visibility_timeout),
                    });
                    received.delivered.push(ReceivedMessage {
                        message_id: message.id,
                        receipt_handle: receipt,
                        body: message.body.clone(),
                        receive_count: message.receive_count,
                        sent_at: message.sent_at,
                        first_received_at,
                        fifo: message.fifo.clone(),
                    });
                    self.receipts.insert(receipt, arrival);
                }
                Fate::DeadLetter => {
                    let message = self
                        .remove(arrival)
                        .expect("a planned arrival is still queued");
                    received.dead_lettered.push(NewMessage {
                        id: message.id,
                        body: message.body,
                        sent_at: message.sent_at,
                        group: message.fifo.map(|placement| placement.group_id),
                        delayed_until: None,
                    });
                }
            }
        }
        received
    }

    /// Examines the messages once each in arrival order and decides what happens to them.
    fn plan_receive(&self, now: Instant, max_messages: u32) -> Vec<(Arrival, Fate)> {
        let mut plan = Vec::new();
        let mut delivered = 0;
        // Groups that still have a message in the queue ahead of the one being examined.
        let mut busy_groups: HashSet<&GroupId> = HashSet::new();
        for (&arrival, message) in &self.messages {
            if delivered == max_messages {
                break;
            }
            let group = message.fifo.as_ref().map(|placement| &placement.group_id);
            let blocked = group.is_some_and(|group| busy_groups.contains(group));
            let eligible = !blocked && message.state(now) == MessageState::Visible;
            let exhausted =
                self.config.dead_letter.as_ref().is_some_and(|dead_letter| {
                    message.receive_count >= dead_letter.max_receive_count
                });
            if eligible && exhausted {
                plan.push((arrival, Fate::DeadLetter));
                continue;
            }
            busy_groups.extend(group);
            if eligible {
                plan.push((arrival, Fate::Deliver));
                delivered += 1;
            }
        }
        plan
    }

    /// The delivery behind `handle`, if it is the latest delivery of a message that is in
    /// flight at `now`.
    fn in_flight_mut(
        &mut self,
        handle: ReceiptHandle,
        now: Instant,
    ) -> Result<(Arrival, &mut Delivery), NotInFlight> {
        let arrival = *self.receipts.get(&handle).ok_or(NotInFlight)?;
        let message = self.messages.get_mut(&arrival).ok_or(NotInFlight)?;
        if message.state(now) != MessageState::InFlight {
            return Err(NotInFlight);
        }
        let delivery = message
            .delivery
            .as_mut()
            .filter(|delivery| delivery.receipt == handle)
            .ok_or(NotInFlight)?;
        Ok((arrival, delivery))
    }

    pub fn delete_in_flight(
        &mut self,
        handle: ReceiptHandle,
        now: Instant,
    ) -> Result<(), NotInFlight> {
        let (arrival, _) = self.in_flight_mut(handle, now)?;
        self.remove(arrival);
        Ok(())
    }

    pub fn change_visibility(
        &mut self,
        handle: ReceiptHandle,
        now: Instant,
        timeout: Seconds,
    ) -> Result<(), NotInFlight> {
        let (_, delivery) = self.in_flight_mut(handle, now)?;
        delivery.invisible_until = now.plus(timeout);
        Ok(())
    }
}
