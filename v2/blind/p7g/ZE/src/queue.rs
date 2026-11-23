//! One queue: its attributes and messages, plus the indexes that keep every
//! operation logarithmic however many messages are held.
//!
//! The indexes are kept consistent through `enter`, `leave` and `take`; no
//! other code touches them.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::ops::Bound;

use serde::Serialize;

use crate::attributes::{Attributes, DeadLetter, RequestedAttributes};
use crate::types::{
    DeduplicationKey, DelaySeconds, GroupId, IdGenerator, MaxMessages, MessageId, ReceiptHandle,
    Time, VisibilityTimeout,
};

/// How long a deduplication id stays taken after it was accepted.
const DEDUPLICATION_WINDOW_SECONDS: u64 = 300;

/// The most messages a queue holds, counting every state.
const MAX_MESSAGES: usize = 100_000;

/// Position of a message in the queue's arrival order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Arrival(u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct SequenceNumber(u64);

/// Where a message is in its life. `until` is the first moment the message is
/// visible again.
#[derive(Debug, Clone, Copy)]
enum Phase {
    Visible,
    Delayed { until: Time },
    InFlight { until: Time, handle: ReceiptHandle },
}

#[derive(Debug)]
pub struct Message {
    id: MessageId,
    body: String,
    sent_at: Time,
    receive_count: u64,
    first_received_at: Option<Time>,
    group: Option<GroupId>,
    sequence: Option<SequenceNumber>,
    phase: Phase,
}

/// The part of a FIFO send that standard queues do not have.
#[derive(Debug)]
pub struct FifoSend {
    pub group: GroupId,
    pub key: DeduplicationKey,
}

#[derive(Debug)]
pub struct NewMessage {
    pub id: MessageId,
    pub body: String,
    pub delay: DelaySeconds,
    pub fifo: Option<FifoSend>,
}

/// A delivered message as it appears in a `ReceiveMessage` result.
#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub struct Delivery {
    message_id: MessageId,
    receipt_handle: ReceiptHandle,
    body: String,
    receive_count: u64,
    sent_at: Time,
    first_received_at: Time,
    #[serde(skip_serializing_if = "Option::is_none")]
    group_id: Option<GroupId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sequence_number: Option<SequenceNumber>,
}

#[derive(Debug, Default)]
pub struct Receipt {
    pub delivered: Vec<Delivery>,
    /// Messages that reached the receive limit; the caller moves them on.
    pub dead_lettered: Vec<Message>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub struct Counts {
    pub visible: usize,
    pub in_flight: usize,
    pub delayed: usize,
}

/// The receipt handle is not the latest handle of a message in flight.
#[derive(Debug, PartialEq, Eq)]
pub struct StaleHandle;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Original {
    pub id: MessageId,
    pub sequence: SequenceNumber,
}

#[derive(Debug, Clone, Copy)]
struct Accepted {
    at: Time,
    original: Original,
}

/// Deduplication ids of a FIFO queue. They outlive the messages they
/// came from, and are forgotten once their window has passed.
#[derive(Debug, Default)]
struct Deduplication {
    accepted: HashMap<DeduplicationKey, Accepted>,
    by_time: VecDeque<(Time, DeduplicationKey)>,
}

impl Deduplication {
    fn find(&self, key: &DeduplicationKey, now: Time) -> Option<Original> {
        self.accepted
            .get(key)
            .filter(|accepted| now.since(accepted.at) < DEDUPLICATION_WINDOW_SECONDS)
            .map(|accepted| accepted.original)
    }

    fn record(&mut self, key: DeduplicationKey, now: Time, original: Original) {
        self.forget_old(now);
        self.by_time.push_back((now, key.clone()));
        self.accepted.insert(key, Accepted { at: now, original });
    }

    fn forget_old(&mut self, now: Time) {
        while self
            .by_time
            .front()
            .is_some_and(|(at, _)| now.since(*at) >= DEDUPLICATION_WINDOW_SECONDS)
        {
            if let Some((at, key)) = self.by_time.pop_front()
                && self.accepted.get(&key).is_some_and(|a| a.at == at)
            {
                self.accepted.remove(&key);
            }
        }
    }
}

#[derive(Debug)]
pub struct Queue {
    attributes: Attributes,
    messages: BTreeMap<Arrival, Message>,
    /// Arrivals of the messages in `Phase::Visible`.
    visible: BTreeSet<Arrival>,
    /// Hidden messages by the moment they become visible again.
    wakeups: BTreeSet<(Time, Arrival)>,
    /// Messages by `SentAt`, oldest first, for retention.
    by_age: BTreeSet<(Time, Arrival)>,
    /// The latest handle of every message in flight.
    handles: HashMap<ReceiptHandle, Arrival>,
    /// Arrivals per FIFO group; the first one is the only one deliverable.
    groups: HashMap<GroupId, BTreeSet<Arrival>>,
    deduplication: Deduplication,
    delayed: usize,
    in_flight: usize,
    next_arrival: u64,
    next_sequence: u64,
}

impl Queue {
    pub fn new(attributes: Attributes) -> Self {
        Self {
            attributes,
            messages: BTreeMap::new(),
            visible: BTreeSet::new(),
            wakeups: BTreeSet::new(),
            by_age: BTreeSet::new(),
            handles: HashMap::new(),
            groups: HashMap::new(),
            deduplication: Deduplication::default(),
            delayed: 0,
            in_flight: 0,
            next_arrival: 0,
            next_sequence: 0,
        }
    }

    pub fn attributes(&self) -> &Attributes {
        &self.attributes
    }

    pub fn is_full(&self) -> bool {
        self.messages.len() >= MAX_MESSAGES
    }

    pub fn counts(&self) -> Counts {
        Counts {
            visible: self.visible.len(),
            in_flight: self.in_flight,
            delayed: self.delayed,
        }
    }

    /// Brings the queue up to `now`: drops messages past retention, then
    /// makes visible every message whose delay or visibility timeout ended.
    /// Must run before anything else looks at the queue.
    pub fn sync(&mut self, now: Time) {
        let retention = self.attributes.retention.get();
        while let Some(&(sent_at, arrival)) = self.by_age.first() {
            if now < sent_at.after(retention) {
                break;
            }
            self.take(arrival);
        }
        while let Some(&(until, arrival)) = self.wakeups.first() {
            if until > now {
                break;
            }
            self.set_phase(arrival, Phase::Visible);
        }
    }

    pub fn update(
        &mut self,
        requested: &RequestedAttributes,
        dead_letter: Option<DeadLetter>,
        now: Time,
    ) {
        self.attributes.update(requested, dead_letter);
        self.sync(now);
    }

    pub fn clear_dead_letter_to(&mut self, deleted: &str) {
        let points_there = self
            .attributes
            .dead_letter
            .as_ref()
            .is_some_and(|d| d.queue.as_str() == deleted);
        if points_there {
            self.attributes.dead_letter = None;
        }
    }

    /// Removes every message. Sequence numbers and deduplication carry on.
    pub fn purge(&mut self) {
        self.messages.clear();
        self.visible.clear();
        self.wakeups.clear();
        self.by_age.clear();
        self.handles.clear();
        self.groups.clear();
        self.delayed = 0;
        self.in_flight = 0;
    }

    /// The message a FIFO send with this key would duplicate.
    pub fn duplicate_of(&self, key: &DeduplicationKey, now: Time) -> Option<Original> {
        self.deduplication.find(key, now)
    }

    /// Adds a message. The result is the sequence number of a FIFO message.
    pub fn accept(&mut self, new: NewMessage, now: Time) -> Option<SequenceNumber> {
        let (group, sequence) = match new.fifo {
            Some(FifoSend { group, key }) => {
                let sequence = self.next_sequence();
                let original = Original {
                    id: new.id,
                    sequence,
                };
                self.deduplication.record(key, now, original);
                (Some(group), Some(sequence))
            }
            None => (None, None),
        };
        let phase = match new.delay.get() {
            0 => Phase::Visible,
            delay => Phase::Delayed {
                until: now.after(delay),
            },
        };
        self.insert(Message {
            id: new.id,
            body: new.body,
            sent_at: now,
            receive_count: 0,
            first_received_at: None,
            group,
            sequence,
            phase,
        });
        sequence
    }

    /// Takes in a message moved from another queue's receive limit. The message
    /// starts over, but keeps its age, so it can be too old to stay.
    pub fn adopt(&mut self, mut message: Message, now: Time) {
        message.receive_count = 0;
        message.first_received_at = None;
        message.phase = Phase::Visible;
        message.sequence = message.group.as_ref().map(|_| self.next_sequence());
        if now >= message.sent_at.after(self.attributes.retention.get()) {
            return;
        }
        self.insert(message);
    }

    /// Delivers up to `max` eligible messages, in arrival order. Messages past
    /// the receive limit are handed back instead of delivered.
    pub fn receive(
        &mut self,
        now: Time,
        max: MaxMessages,
        visibility: VisibilityTimeout,
        ids: &mut IdGenerator,
    ) -> Receipt {
        let limit = self
            .attributes
            .dead_letter
            .as_ref()
            .map(|d| u64::from(d.max_receive_count.get()));
        let mut receipt = Receipt::default();
        let mut remaining = max.get();
        let mut cursor = Bound::Unbounded;
        while remaining > 0 {
            let Some(&arrival) = self.visible.range((cursor, Bound::Unbounded)).next() else {
                break;
            };
            cursor = Bound::Excluded(arrival);
            if !self.is_head_of_group(arrival) {
                continue;
            }
            if limit.is_some_and(|limit| self.message(arrival).receive_count >= limit) {
                receipt.dead_lettered.push(self.take(arrival));
                continue;
            }
            receipt
                .delivered
                .push(self.deliver(arrival, now, visibility, ids.next_receipt()));
            remaining -= 1;
        }
        receipt
    }

    pub fn delete_in_flight(&mut self, handle: ReceiptHandle) -> Result<(), StaleHandle> {
        let arrival = self.in_flight_arrival(handle)?;
        self.take(arrival);
        Ok(())
    }

    pub fn change_visibility(
        &mut self,
        handle: ReceiptHandle,
        visibility: VisibilityTimeout,
        now: Time,
    ) -> Result<(), StaleHandle> {
        let arrival = self.in_flight_arrival(handle)?;
        let phase = match visibility.get() {
            0 => Phase::Visible,
            seconds => Phase::InFlight {
                until: now.after(seconds),
                handle,
            },
        };
        self.set_phase(arrival, phase);
        Ok(())
    }

    fn in_flight_arrival(&self, handle: ReceiptHandle) -> Result<Arrival, StaleHandle> {
        self.handles.get(&handle).copied().ok_or(StaleHandle)
    }

    fn next_sequence(&mut self) -> SequenceNumber {
        self.next_sequence += 1;
        SequenceNumber(self.next_sequence)
    }

    fn message(&self, arrival: Arrival) -> &Message {
        self.messages
            .get(&arrival)
            .expect("arrivals in the indexes belong to stored messages")
    }

    fn message_mut(&mut self, arrival: Arrival) -> &mut Message {
        self.messages
            .get_mut(&arrival)
            .expect("arrivals in the indexes belong to stored messages")
    }

    /// A message of a FIFO queue may only be delivered when no earlier
    /// message of its group is still in the queue.
    fn is_head_of_group(&self, arrival: Arrival) -> bool {
        match &self.message(arrival).group {
            None => true,
            Some(group) => self.groups.get(group).and_then(BTreeSet::first) == Some(&arrival),
        }
    }

    fn deliver(
        &mut self,
        arrival: Arrival,
        now: Time,
        visibility: VisibilityTimeout,
        handle: ReceiptHandle,
    ) -> Delivery {
        let message = self.message_mut(arrival);
        message.receive_count += 1;
        let first_received_at = *message.first_received_at.get_or_insert(now);
        let delivery = Delivery {
            message_id: message.id,
            receipt_handle: handle,
            body: message.body.clone(),
            receive_count: message.receive_count,
            sent_at: message.sent_at,
            first_received_at,
            group_id: message.group.clone(),
            sequence_number: message.sequence,
        };
        // With a zero timeout the message stays visible and the handle is dead.
        if visibility.get() > 0 {
            let until = now.after(visibility.get());
            self.set_phase(arrival, Phase::InFlight { until, handle });
        }
        delivery
    }

    fn insert(&mut self, message: Message) {
        let arrival = Arrival(self.next_arrival);
        self.next_arrival += 1;
        self.by_age.insert((message.sent_at, arrival));
        if let Some(group) = &message.group {
            self.groups
                .entry(group.clone())
                .or_default()
                .insert(arrival);
        }
        let phase = message.phase;
        self.messages.insert(arrival, message);
        self.enter(arrival, phase);
    }

    /// Removes a stored message from every index and returns it.
    fn take(&mut self, arrival: Arrival) -> Message {
        let message = self
            .messages
            .remove(&arrival)
            .expect("arrivals in the indexes belong to stored messages");
        self.by_age.remove(&(message.sent_at, arrival));
        self.leave(arrival, message.phase);
        if let Some(group) = &message.group
            && let Some(arrivals) = self.groups.get_mut(group)
        {
            arrivals.remove(&arrival);
            if arrivals.is_empty() {
                self.groups.remove(group);
            }
        }
        message
    }

    fn set_phase(&mut self, arrival: Arrival, phase: Phase) {
        let previous = std::mem::replace(&mut self.message_mut(arrival).phase, phase);
        self.leave(arrival, previous);
        self.enter(arrival, phase);
    }

    fn enter(&mut self, arrival: Arrival, phase: Phase) {
        match phase {
            Phase::Visible => {
                self.visible.insert(arrival);
            }
            Phase::Delayed { until } => {
                self.wakeups.insert((until, arrival));
                self.delayed += 1;
            }
            Phase::InFlight { until, handle } => {
                self.wakeups.insert((until, arrival));
                self.handles.insert(handle, arrival);
                self.in_flight += 1;
            }
        }
    }

    fn leave(&mut self, arrival: Arrival, phase: Phase) {
        match phase {
            Phase::Visible => {
                self.visible.remove(&arrival);
            }
            Phase::Delayed { until } => {
                self.wakeups.remove(&(until, arrival));
                self.delayed -= 1;
            }
            Phase::InFlight { until, handle } => {
                self.wakeups.remove(&(until, arrival));
                self.handles.remove(&handle);
                self.in_flight -= 1;
            }
        }
    }
}
