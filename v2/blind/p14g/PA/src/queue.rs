//! One queue: its attributes, its messages in arrival order, and the
//! indexes that keep the hot operations off a full scan.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use serde::Serialize;

use crate::attributes::Attributes;
use crate::types::{
    Body, GroupId, MaxMessages, MessageId, QueueName, ReceiptHandle, ReceiptHandleIssuer,
    SequenceNumber, Time, VisibilityTimeout,
};

/// Seconds within which a repeated FIFO deduplication id is a duplicate.
const DEDUPLICATION_WINDOW: u64 = 300;

/// Why a message is not visible, if it is not.
#[derive(Clone, Copy, Debug)]
pub enum Hold {
    /// Sent with a delay; visible once `until` is reached.
    Delayed { until: Time },
    /// Delivered; visible again once `until` is reached. `handle` is the
    /// latest receipt handle of the message.
    InFlight { until: Time, handle: ReceiptHandle },
    /// Nothing holds the message back.
    Released,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Visible,
    InFlight,
    Delayed,
}

#[derive(Debug)]
pub struct Message {
    pub id: MessageId,
    pub body: Body,
    pub sent_at: Time,
    pub receive_count: u64,
    pub first_received_at: Option<Time>,
    pub group: Option<GroupId>,
    pub hold: Hold,
}

impl Message {
    pub fn state(&self, now: Time) -> State {
        match self.hold {
            Hold::Delayed { until } if now < until => State::Delayed,
            Hold::InFlight { until, .. } if now < until => State::InFlight,
            Hold::Delayed { .. } | Hold::InFlight { .. } | Hold::Released => State::Visible,
        }
    }
}

/// The number of messages in each state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Counts {
    pub visible: u64,
    pub in_flight: u64,
    pub delayed: u64,
}

/// A message as `ReceiveMessage` reports it.
#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Delivery {
    pub message_id: MessageId,
    pub receipt_handle: ReceiptHandle,
    pub body: Body,
    pub receive_count: u64,
    pub sent_at: Time,
    pub first_received_at: Time,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_id: Option<GroupId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sequence_number: Option<SequenceNumber>,
}

/// What one `ReceiveMessage` did to a queue.
#[derive(Debug)]
pub struct Received {
    pub deliveries: Vec<Delivery>,
    /// Messages that left the queue for its dead-letter queue, with that
    /// queue's name, in the order they were examined.
    pub dead_lettered: Option<(QueueName, Vec<Message>)>,
}

/// The first accepted FIFO send of a deduplication id in the current window.
#[derive(Debug)]
struct Accepted {
    at: Time,
    id: MessageId,
    sequence: SequenceNumber,
}

#[derive(Debug)]
pub struct Queue {
    pub attributes: Attributes,
    /// Keyed by arrival position, so iteration is arrival order.
    messages: BTreeMap<SequenceNumber, Message>,
    /// `(sent_at, position)` of every message, to expire the oldest first.
    by_sent: BTreeSet<(Time, SequenceNumber)>,
    /// Latest receipt handle of each message that has been delivered.
    handles: HashMap<ReceiptHandle, SequenceNumber>,
    /// Deduplication ids outlive their messages, so they are kept apart.
    deduplication: HashMap<String, Accepted>,
    next_sequence: SequenceNumber,
}

impl Queue {
    pub fn new(attributes: Attributes) -> Self {
        Self {
            attributes,
            messages: BTreeMap::new(),
            by_sent: BTreeSet::new(),
            handles: HashMap::new(),
            deduplication: HashMap::new(),
            next_sequence: SequenceNumber::FIRST,
        }
    }

    /// Number of messages held, in any state.
    pub fn message_count(&self) -> usize {
        self.messages.len()
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

    /// Removes every message whose retention has run out. Called before a
    /// queue is read or changed, which is the only way to observe expiry.
    pub fn expire(&mut self, now: Time) {
        let retention = self.attributes.retention_seconds.get();
        while let Some(&(sent_at, position)) = self.by_sent.first() {
            if sent_at.after(retention) > now {
                break;
            }
            self.remove(position);
        }
    }

    /// Removes all messages. Sequence numbers and deduplication carry on.
    pub fn purge(&mut self) {
        self.messages.clear();
        self.by_sent.clear();
        self.handles.clear();
    }

    /// Adds `message` at the end of the queue and returns its position.
    pub fn insert(&mut self, message: Message) -> SequenceNumber {
        let position = self.next_sequence;
        self.next_sequence = position.next();
        self.by_sent.insert((message.sent_at, position));
        self.messages.insert(position, message);
        position
    }

    fn remove(&mut self, position: SequenceNumber) -> Option<Message> {
        let message = self.messages.remove(&position)?;
        self.by_sent.remove(&(message.sent_at, position));
        if let Hold::InFlight { handle, .. } = message.hold {
            self.handles.remove(&handle);
        }
        Some(message)
    }

    /// The original send of `key` if it was accepted less than the
    /// deduplication window ago.
    pub fn duplicate_of(&self, key: &str, now: Time) -> Option<(MessageId, SequenceNumber)> {
        self.deduplication
            .get(key)
            .filter(|accepted| now.since(accepted.at) < DEDUPLICATION_WINDOW)
            .map(|accepted| (accepted.id, accepted.sequence))
    }

    /// Opens a new deduplication window for `key`.
    pub fn record_deduplication(
        &mut self,
        key: String,
        now: Time,
        id: MessageId,
        sequence: SequenceNumber,
    ) {
        self.deduplication.insert(
            key,
            Accepted {
                at: now,
                id,
                sequence,
            },
        );
    }

    /// Adds a message that was moved here from another queue. It starts over
    /// as a fresh, undelayed message but keeps its `SentAt`, so it may be
    /// expired on arrival.
    pub fn insert_dead_lettered(&mut self, mut message: Message, now: Time) {
        message.receive_count = 0;
        message.first_received_at = None;
        message.hold = Hold::Released;
        self.insert(message);
        self.expire(now);
    }

    /// Delivers up to `max` eligible messages in arrival order and moves
    /// the ones that exhausted their receive count out of the queue.
    pub fn receive(
        &mut self,
        now: Time,
        max: MaxMessages,
        visibility_timeout: VisibilityTimeout,
        issuer: &mut ReceiptHandleIssuer,
    ) -> Received {
        let (to_deliver, to_move) = self.plan_receive(now, max);

        let mut deliveries = Vec::with_capacity(to_deliver.len());
        for position in to_deliver {
            let handle = issuer.issue();
            let message = self
                .messages
                .get_mut(&position)
                .expect("the plan only names positions present in the queue");
            if let Hold::InFlight {
                handle: previous, ..
            } = message.hold
            {
                self.handles.remove(&previous);
            }
            message.receive_count += 1;
            let first_received_at = *message.first_received_at.get_or_insert(now);
            message.hold = Hold::InFlight {
                until: now.after(visibility_timeout.get()),
                handle,
            };
            self.handles.insert(handle, position);
            deliveries.push(Delivery {
                message_id: message.id,
                receipt_handle: handle,
                body: message.body.clone(),
                receive_count: message.receive_count,
                sent_at: message.sent_at,
                first_received_at,
                group_id: message.group.clone(),
                sequence_number: self.attributes.kind.is_fifo().then_some(position),
            });
        }

        let moved: Vec<Message> = to_move
            .into_iter()
            .map(|position| {
                self.remove(position)
                    .expect("the plan only names positions present in the queue")
            })
            .collect();
        let dead_lettered = match &self.attributes.dead_letter {
            Some(dead_letter) if !moved.is_empty() => Some((dead_letter.queue.clone(), moved)),
            _ => None,
        };
        Received {
            deliveries,
            dead_lettered,
        }
    }

    /// Positions to deliver and positions to dead-letter, both in arrival
    /// order. Decisions only depend on the state before the call.
    fn plan_receive(
        &self,
        now: Time,
        max: MaxMessages,
    ) -> (Vec<SequenceNumber>, Vec<SequenceNumber>) {
        let limit = self
            .attributes
            .dead_letter
            .as_ref()
            .map(|dead_letter| dead_letter.max_receive_count.get());
        let mut to_deliver = Vec::new();
        let mut to_move = Vec::new();
        // Groups with an earlier message that stays in the queue.
        let mut blocked: HashSet<&GroupId> = HashSet::new();
        let mut delivering = 0;
        for (&position, message) in &self.messages {
            if delivering == max.get() {
                break;
            }
            if let Some(group) = &message.group
                && !blocked.insert(group)
            {
                continue;
            }
            if message.state(now) != State::Visible {
                continue;
            }
            if limit.is_some_and(|limit| message.receive_count >= limit) {
                to_move.push(position);
                if let Some(group) = &message.group {
                    blocked.remove(group);
                }
            } else {
                to_deliver.push(position);
                delivering += 1;
            }
        }
        (to_deliver, to_move)
    }

    fn in_flight_position(&self, handle: ReceiptHandle, now: Time) -> Option<SequenceNumber> {
        let position = *self.handles.get(&handle)?;
        let message = self.messages.get(&position)?;
        (message.state(now) == State::InFlight).then_some(position)
    }

    /// Removes the message `handle` is the latest handle of, if it is in
    /// flight at `now`.
    pub fn take_in_flight(&mut self, handle: ReceiptHandle, now: Time) -> Option<Message> {
        let position = self.in_flight_position(handle, now)?;
        self.remove(position)
    }

    /// Sets the end of the visibility timeout of the message `handle` is the
    /// latest handle of, if it is in flight at `now`.
    pub fn change_visibility(
        &mut self,
        handle: ReceiptHandle,
        now: Time,
        timeout: VisibilityTimeout,
    ) -> Option<()> {
        let position = self.in_flight_position(handle, now)?;
        let message = self.messages.get_mut(&position)?;
        message.hold = Hold::InFlight {
            until: now.after(timeout.get()),
            handle,
        };
        Some(())
    }
}
