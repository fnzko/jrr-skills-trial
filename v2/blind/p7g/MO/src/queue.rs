//! A single queue: its messages in arrival order and everything that
//! depends only on that queue.

use std::collections::{BTreeMap, HashMap, HashSet};

use serde::Serialize;

use crate::attributes::{DeadLetter, QueueAttributes, Update};
use crate::error::{Error, ErrorCode, Result};
use crate::types::{
    Body, DEDUPLICATION_WINDOW, DedupKey, DelaySeconds, GroupId, MaxMessages, MessageId, QueueName,
    ReceiptHandle, SequenceNumber, Time, VisibilityTimeout,
};

/// Position of a message among all messages added to one queue. Unlike a
/// vector index it stays valid when other messages leave.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Arrival(u64);

/// Arrivals come from a scan of the same map moments earlier, with no
/// mutation in between that could remove them.
const PLANNED: &str = "arrival was found in this queue just before";

/// Why a message that is not yet visible is hidden, and until when.
#[derive(Debug, Clone, Copy)]
enum Hold {
    Free,
    Delayed { until: Time },
    InFlight { until: Time, handle: ReceiptHandle },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Visible,
    InFlight,
    Delayed,
}

#[derive(Debug)]
struct FifoPlacement {
    group: GroupId,
    sequence: SequenceNumber,
}

/// A message that left its queue because of too many deliveries.
#[derive(Debug)]
pub struct DeadMessage(Message);

#[derive(Debug)]
struct Message {
    id: MessageId,
    body: Body,
    sent_at: Time,
    fifo: Option<FifoPlacement>,
    receive_count: u32,
    first_received_at: Option<Time>,
    hold: Hold,
}

impl Message {
    fn state(&self, now: Time) -> State {
        match self.hold {
            Hold::Delayed { until } if now < until => State::Delayed,
            Hold::InFlight { until, .. } if now < until => State::InFlight,
            Hold::Free | Hold::Delayed { .. } | Hold::InFlight { .. } => State::Visible,
        }
    }

    fn group(&self) -> Option<&GroupId> {
        self.fifo.as_ref().map(|f| &f.group)
    }

    fn handle(&self) -> Option<ReceiptHandle> {
        match self.hold {
            Hold::InFlight { handle, .. } => Some(handle),
            Hold::Free | Hold::Delayed { .. } => None,
        }
    }
}

/// The `Counts` object of `GetQueueAttributes`.
#[derive(Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Counts {
    visible: usize,
    in_flight: usize,
    delayed: usize,
}

/// One entry of the `Messages` array of `ReceiveMessage`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Delivery {
    message_id: MessageId,
    receipt_handle: ReceiptHandle,
    body: Body,
    receive_count: u32,
    sent_at: Time,
    first_received_at: Time,
    #[serde(skip_serializing_if = "Option::is_none")]
    group_id: Option<GroupId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sequence_number: Option<SequenceNumber>,
}

/// What one `ReceiveMessage` pass did to a queue.
#[derive(Debug)]
pub struct Received {
    pub delivered: Vec<Delivery>,
    pub dead: Vec<DeadMessage>,
}

/// A send that passed all checks and is ready to be stored.
#[derive(Debug)]
pub struct Accept {
    pub id: MessageId,
    pub body: Body,
    pub delay: DelaySeconds,
    pub fifo: Option<FifoAccept>,
}

#[derive(Debug)]
pub struct FifoAccept {
    pub group: GroupId,
    pub deduplication: DedupKey,
}

/// The outcome of an accepted FIFO send, remembered for deduplication.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Original {
    pub id: MessageId,
    pub sequence: SequenceNumber,
    accepted_at: Time,
}

#[derive(Debug)]
pub struct Queue {
    attributes: QueueAttributes,
    messages: BTreeMap<Arrival, Message>,
    next_arrival: Arrival,
    next_sequence: SequenceNumber,
    /// Latest handle of each message; an entry can be stale, so lookups
    /// confirm it against the message itself.
    receipts: HashMap<ReceiptHandle, Arrival>,
    deduplication: HashMap<DedupKey, Original>,
}

impl Queue {
    pub const MAX_MESSAGES: usize = 100_000;

    pub fn new(attributes: QueueAttributes) -> Self {
        Self {
            attributes,
            messages: BTreeMap::new(),
            next_arrival: Arrival(1),
            next_sequence: SequenceNumber::FIRST,
            receipts: HashMap::new(),
            deduplication: HashMap::new(),
        }
    }

    pub fn attributes(&self) -> &QueueAttributes {
        &self.attributes
    }

    pub fn is_full(&self) -> bool {
        self.messages.len() >= Self::MAX_MESSAGES
    }

    pub fn counts(&self, now: Time) -> Counts {
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

    pub fn update(&mut self, update: Update, now: Time) {
        self.attributes.apply(update);
        self.expire(now);
    }

    /// Called when the dead-letter queue `gone` is deleted.
    pub fn forget_dead_letter_queue(&mut self, gone: &QueueName) {
        if self
            .attributes
            .dead_letter
            .as_ref()
            .is_some_and(|d| &d.queue == gone)
        {
            self.attributes.dead_letter = None;
        }
    }

    /// Drops every message that has outlived the retention period.
    pub fn expire(&mut self, now: Time) {
        let retention = self.attributes.retention;
        let Self {
            messages, receipts, ..
        } = self;
        messages.retain(|_, message| {
            let keep = now < message.sent_at.plus(retention);
            if let Some(handle) = message.handle().filter(|_| !keep) {
                receipts.remove(&handle);
            }
            keep
        });
    }

    pub fn purge(&mut self) {
        self.messages.clear();
        self.receipts.clear();
    }

    /// The accepted send that makes a send with `key` at `now` a duplicate.
    pub fn original_of(&self, key: &DedupKey, now: Time) -> Option<Original> {
        self.deduplication
            .get(key)
            .filter(|o| now.since(o.accepted_at) < DEDUPLICATION_WINDOW)
            .copied()
    }

    /// Stores a message at the end of the queue. The sequence number is
    /// returned for FIFO sends only.
    pub fn accept(&mut self, accept: Accept, now: Time) -> Option<SequenceNumber> {
        let hold = match accept.delay.get() {
            0 => Hold::Free,
            _ => Hold::Delayed {
                until: now.plus(accept.delay),
            },
        };
        let (group, key) = match accept.fifo {
            Some(f) => (Some(f.group), Some(f.deduplication)),
            None => (None, None),
        };
        let message = Message {
            id: accept.id,
            body: accept.body,
            sent_at: now,
            fifo: None,
            receive_count: 0,
            first_received_at: None,
            hold,
        };
        let (_, sequence) = self.append(message, group);
        if let (Some(key), Some(sequence)) = (key, sequence) {
            let original = Original {
                id: accept.id,
                sequence,
                accepted_at: now,
            };
            self.deduplication.insert(key, original);
        }
        sequence
    }

    /// Adds a message that was dead-lettered from another queue. It starts
    /// over: no deliveries, no delay, and a sequence number of its own.
    pub fn admit(&mut self, dead: DeadMessage, now: Time) {
        let DeadMessage(mut message) = dead;
        message.receive_count = 0;
        message.first_received_at = None;
        message.hold = Hold::Free;
        let group = message.fifo.take().map(|f| f.group);
        let too_old = now >= message.sent_at.plus(self.attributes.retention);
        let (arrival, _) = self.append(message, group);
        if too_old {
            self.remove(arrival);
        }
    }

    fn append(
        &mut self,
        mut message: Message,
        group: Option<GroupId>,
    ) -> (Arrival, Option<SequenceNumber>) {
        let sequence = group.map(|group| {
            let sequence = self.next_sequence;
            self.next_sequence = sequence.next();
            message.fifo = Some(FifoPlacement { group, sequence });
            sequence
        });
        let arrival = self.next_arrival;
        self.next_arrival = Arrival(arrival.0 + 1);
        self.messages.insert(arrival, message);
        (arrival, sequence)
    }

    fn remove(&mut self, arrival: Arrival) -> Option<Message> {
        let message = self.messages.remove(&arrival)?;
        if let Some(handle) = message.handle() {
            self.receipts.remove(&handle);
        }
        Some(message)
    }

    /// Delivers up to `max` messages and moves exhausted ones out, scanning
    /// in arrival order exactly once.
    pub fn receive(
        &mut self,
        now: Time,
        max: MaxMessages,
        visibility: VisibilityTimeout,
        issue: &mut impl FnMut() -> ReceiptHandle,
    ) -> Received {
        let plan = self.plan_receive(now, max);
        let mut received = Received {
            delivered: Vec::new(),
            dead: Vec::new(),
        };
        for (arrival, step) in plan {
            match step {
                Step::Deliver => {
                    let delivery = self.deliver(arrival, now, visibility, issue());
                    received.delivered.push(delivery);
                }
                Step::DeadLetter => {
                    let message = self.remove(arrival).expect(PLANNED);
                    received.dead.push(DeadMessage(message));
                }
            }
        }
        received
    }

    fn plan_receive(&self, now: Time, max: MaxMessages) -> Vec<(Arrival, Step)> {
        let limit = self
            .attributes
            .dead_letter
            .as_ref()
            .map(|d| d.max_receive_count.get());
        let max = usize::try_from(max.get()).unwrap_or(usize::MAX);
        let mut plan = Vec::new();
        let mut delivered = 0;
        let mut seen_groups: HashSet<&GroupId> = HashSet::new();
        for (&arrival, message) in &self.messages {
            if delivered == max {
                break;
            }
            let group = message.group();
            let earlier_in_group = group.is_some_and(|g| seen_groups.contains(g));
            if earlier_in_group || message.state(now) != State::Visible {
                seen_groups.extend(group);
                continue;
            }
            if limit.is_some_and(|limit| message.receive_count >= limit) {
                plan.push((arrival, Step::DeadLetter));
                continue;
            }
            seen_groups.extend(group);
            plan.push((arrival, Step::Deliver));
            delivered += 1;
        }
        plan
    }

    fn deliver(
        &mut self,
        arrival: Arrival,
        now: Time,
        visibility: VisibilityTimeout,
        handle: ReceiptHandle,
    ) -> Delivery {
        let message = self.messages.get_mut(&arrival).expect(PLANNED);
        if let Some(previous) = message.handle() {
            self.receipts.remove(&previous);
        }
        message.receive_count += 1;
        let first_received_at = *message.first_received_at.get_or_insert(now);
        message.hold = Hold::InFlight {
            until: now.plus(visibility),
            handle,
        };
        self.receipts.insert(handle, arrival);
        Delivery {
            message_id: message.id,
            receipt_handle: handle,
            body: message.body.clone(),
            receive_count: message.receive_count,
            sent_at: message.sent_at,
            first_received_at,
            group_id: message.fifo.as_ref().map(|f| f.group.clone()),
            sequence_number: message.fifo.as_ref().map(|f| f.sequence),
        }
    }

    /// The arrival of the message for which `handle` is the latest handle
    /// and which is in flight at `now`.
    fn in_flight(&self, handle: ReceiptHandle, now: Time) -> Result<Arrival> {
        let arrival = self.receipts.get(&handle).copied();
        arrival
            .filter(|a| {
                self.messages
                    .get(a)
                    .is_some_and(|m| m.handle() == Some(handle) && m.state(now) == State::InFlight)
            })
            .ok_or_else(|| {
                Error::new(
                    ErrorCode::ReceiptHandleExpired,
                    format!("receipt handle {handle} is no longer usable in this queue"),
                )
            })
    }

    pub fn delete(&mut self, handle: ReceiptHandle, now: Time) -> Result<()> {
        let arrival = self.in_flight(handle, now)?;
        self.remove(arrival);
        Ok(())
    }

    pub fn change_visibility(
        &mut self,
        handle: ReceiptHandle,
        timeout: VisibilityTimeout,
        now: Time,
    ) -> Result<()> {
        let arrival = self.in_flight(handle, now)?;
        let message = self.messages.get_mut(&arrival).expect(PLANNED);
        message.hold = Hold::InFlight {
            until: now.plus(timeout),
            handle,
        };
        Ok(())
    }

    pub fn dead_letter(&self) -> Option<&DeadLetter> {
        self.attributes.dead_letter.as_ref()
    }
}

#[derive(Debug, Clone, Copy)]
enum Step {
    Deliver,
    DeadLetter,
}
