//! A queue and its messages (SPEC sections 4 and 6).

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use serde::Serialize;

use crate::attributes::Attributes;
use crate::handles::{HandleLog, HandleOwner};
use crate::types::{
    DedupId, GroupId, MessageId, QueueId, QueueName, ReceiptHandle, Seconds, SequenceNumber, Slot,
    Time,
};

pub const MAX_MESSAGES_PER_QUEUE: usize = 100_000;
const DEDUP_WINDOW: Seconds = Seconds::new(300);

/// Why a message is not simply visible.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Hold {
    None,
    DelayedUntil(Time),
    InFlightUntil(Time),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Visible,
    InFlight,
    Delayed,
}

#[derive(Debug, Clone)]
struct FifoPlacement {
    group: GroupId,
    sequence: SequenceNumber,
}

#[derive(Debug, Clone)]
pub struct Message {
    id: MessageId,
    body: String,
    sent_at: Time,
    hold: Hold,
    receive_count: u32,
    first_received_at: Option<Time>,
    fifo: Option<FifoPlacement>,
    /// The handle of the latest delivery.
    handle: Option<ReceiptHandle>,
}

impl Message {
    fn state(&self, now: Time) -> State {
        match self.hold {
            Hold::DelayedUntil(until) if now < until => State::Delayed,
            Hold::InFlightUntil(until) if now < until => State::InFlight,
            _ => State::Visible,
        }
    }
}

/// A message as `ReceiveMessage` reports it.
#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Delivery {
    message_id: MessageId,
    receipt_handle: ReceiptHandle,
    body: String,
    receive_count: u32,
    sent_at: Time,
    first_received_at: Time,
    #[serde(skip_serializing_if = "Option::is_none")]
    group_id: Option<GroupId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sequence_number: Option<SequenceNumber>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub struct Counts {
    pub visible: usize,
    pub in_flight: usize,
    pub delayed: usize,
}

/// What a send produced: the message, and its sequence number in a FIFO queue.
#[derive(Debug, Clone, Copy)]
pub struct Accepted {
    pub id: MessageId,
    pub sequence: Option<SequenceNumber>,
    at: Time,
}

/// The outcome of one `ReceiveMessage` on a queue.
#[derive(Debug)]
pub struct Received {
    pub deliveries: Vec<Delivery>,
    /// Messages that must be added to the dead-letter queue, in order.
    pub dead: Vec<Message>,
}

enum Step {
    Deliver(Slot),
    Kill(Slot),
}

#[derive(Debug)]
pub struct Queue {
    id: QueueId,
    name: QueueName,
    attributes: Attributes,
    messages: BTreeMap<Slot, Message>,
    next_slot: Slot,
    /// Message slots ordered by send time, so retention only looks at the oldest.
    by_age: BTreeSet<(Time, Slot)>,
    next_sequence: SequenceNumber,
    /// Deduplication ids of FIFO sends. They outlive their messages.
    accepted: HashMap<DedupId, Accepted>,
}

impl Queue {
    pub fn new(id: QueueId, name: QueueName, attributes: Attributes) -> Self {
        Self {
            id,
            name,
            attributes,
            messages: BTreeMap::new(),
            next_slot: Slot::FIRST,
            by_age: BTreeSet::new(),
            next_sequence: SequenceNumber::FIRST,
            accepted: HashMap::new(),
        }
    }

    pub fn name(&self) -> &QueueName {
        &self.name
    }

    pub fn attributes(&self) -> &Attributes {
        &self.attributes
    }

    pub fn attributes_mut(&mut self) -> &mut Attributes {
        &mut self.attributes
    }

    pub fn is_full(&self) -> bool {
        self.messages.len() >= MAX_MESSAGES_PER_QUEUE
    }

    /// Removes the messages whose retention has run out.
    pub fn expire(&mut self, now: Time) {
        let retention = self.attributes.retention;
        while let Some(&(sent_at, slot)) = self.by_age.first() {
            if now < sent_at + retention {
                break;
            }
            self.by_age.pop_first();
            self.messages.remove(&slot);
        }
    }

    pub fn counts(&self, now: Time) -> Counts {
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

    /// Removes every message. Sequence numbers and deduplication carry on.
    pub fn purge(&mut self) {
        self.messages.clear();
        self.by_age.clear();
    }

    /// The earlier send with this deduplication id if it still blocks a new one.
    pub fn duplicate_of(&self, dedup: &DedupId, now: Time) -> Option<Accepted> {
        self.accepted
            .get(dedup)
            .filter(|earlier| now < earlier.at + DEDUP_WINDOW)
            .copied()
    }

    /// Adds a new message. A FIFO send names its group and deduplication id.
    pub fn accept(
        &mut self,
        id: MessageId,
        body: String,
        now: Time,
        delay: Seconds,
        fifo: Option<(GroupId, DedupId)>,
    ) -> Accepted {
        let (placement, dedup) = match fifo {
            Some((group, dedup)) => (
                Some(FifoPlacement {
                    group,
                    sequence: self.take_sequence(),
                }),
                Some(dedup),
            ),
            None => (None, None),
        };
        let accepted = Accepted {
            id,
            sequence: placement.as_ref().map(|p| p.sequence),
            at: now,
        };
        if let Some(dedup) = dedup {
            self.accepted.insert(dedup, accepted);
        }
        self.insert(Message {
            id,
            body,
            sent_at: now,
            hold: Hold::DelayedUntil(now + delay),
            receive_count: 0,
            first_received_at: None,
            fifo: placement,
            handle: None,
        });
        accepted
    }

    /// Adds a message moved from another queue: visible at once, as if never received.
    pub fn admit_dead(&mut self, mut message: Message, now: Time) {
        message.hold = Hold::None;
        message.receive_count = 0;
        message.first_received_at = None;
        message.handle = None;
        if message.fifo.is_some() {
            let sequence = self.take_sequence();
            if let Some(placement) = message.fifo.as_mut() {
                placement.sequence = sequence;
            }
        }
        self.insert(message);
        self.expire(now);
    }

    fn take_sequence(&mut self) -> SequenceNumber {
        let sequence = self.next_sequence;
        self.next_sequence = sequence.next();
        sequence
    }

    fn insert(&mut self, message: Message) {
        let slot = self.next_slot;
        self.next_slot = slot.next();
        self.by_age.insert((message.sent_at, slot));
        self.messages.insert(slot, message);
    }

    fn remove(&mut self, slot: Slot) -> Option<Message> {
        let message = self.messages.remove(&slot)?;
        self.by_age.remove(&(message.sent_at, slot));
        Some(message)
    }

    /// Delivers up to `max` eligible messages in arrival order. Eligible messages that reached
    /// the queue's `MaxReceiveCount` are removed and handed back in `dead` instead.
    pub fn receive(
        &mut self,
        now: Time,
        max: u32,
        visibility_timeout: Seconds,
        handles: &mut HandleLog,
    ) -> Received {
        let steps = self.plan_receive(now, max);
        let mut received = Received {
            deliveries: Vec::new(),
            dead: Vec::new(),
        };
        for step in steps {
            match step {
                Step::Kill(slot) => received.dead.extend(self.remove(slot)),
                Step::Deliver(slot) => {
                    let owner = HandleOwner {
                        queue: self.id,
                        slot,
                    };
                    if let Some(message) = self.messages.get_mut(&slot) {
                        let handle = handles.issue(owner);
                        message.receive_count += 1;
                        let first = *message.first_received_at.get_or_insert(now);
                        message.hold = Hold::InFlightUntil(now + visibility_timeout);
                        message.handle = Some(handle);
                        received.deliveries.push(Delivery {
                            message_id: message.id,
                            receipt_handle: handle,
                            body: message.body.clone(),
                            receive_count: message.receive_count,
                            sent_at: message.sent_at,
                            first_received_at: first,
                            group_id: message.fifo.as_ref().map(|p| p.group.clone()),
                            sequence_number: message.fifo.as_ref().map(|p| p.sequence),
                        });
                    }
                }
            }
        }
        received
    }

    /// Examines the messages once in arrival order, without changing anything.
    fn plan_receive(&self, now: Time, max: u32) -> Vec<Step> {
        let dead_after = self
            .attributes
            .dead_letter
            .as_ref()
            .map(|d| d.max_receive_count());
        let mut groups_in_queue: HashSet<&GroupId> = HashSet::new();
        let mut steps = Vec::new();
        let mut delivered = 0;
        for (&slot, message) in &self.messages {
            if delivered == max {
                break;
            }
            let group = message.fifo.as_ref().map(|p| &p.group);
            let blocked = group.is_some_and(|g| groups_in_queue.contains(g));
            let eligible = !blocked && message.state(now) == State::Visible;
            if eligible && dead_after.is_some_and(|limit| message.receive_count >= limit) {
                steps.push(Step::Kill(slot));
                continue;
            }
            if let Some(group) = group {
                groups_in_queue.insert(group);
            }
            if eligible {
                steps.push(Step::Deliver(slot));
                delivered += 1;
            }
        }
        steps
    }

    /// Whether `handle` is the latest handle of a message in flight in this queue.
    pub fn holds_in_flight(&self, owner: HandleOwner, handle: ReceiptHandle, now: Time) -> bool {
        owner.queue == self.id
            && self.messages.get(&owner.slot).is_some_and(|message| {
                message.handle == Some(handle) && message.state(now) == State::InFlight
            })
    }

    pub fn delete(&mut self, slot: Slot) {
        self.remove(slot);
    }

    /// Makes the message in `slot` in flight until `until`.
    pub fn hold_until(&mut self, slot: Slot, until: Time) {
        if let Some(message) = self.messages.get_mut(&slot) {
            message.hold = Hold::InFlightUntil(until);
        }
    }
}
