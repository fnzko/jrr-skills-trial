//! One queue and everything that only needs that queue: its messages, their
//! states, retention, deduplication, delivery and receipt handles.
//!
//! All times are virtual seconds on the broker's clock.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use serde::Serialize;

use crate::error::{Error, ErrorCode};
use crate::ids::{IdSource, MessageId, ReceiptHandle};
use crate::names::GroupId;
use crate::request::{AttributePatch, NewMessage};

pub(crate) const MAX_MESSAGES_PER_QUEUE: usize = 100_000;

/// A send is a duplicate while less than this many seconds have passed since
/// the send that opened the window.
const DEDUPLICATION_WINDOW: u64 = 300;

/// Where a queue sends messages that were received too often.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct DeadLetter {
    /// Names an existing queue: the broker checks this when the configuration
    /// is set and clears it when that queue is deleted.
    #[serde(rename = "DeadLetterQueue")]
    pub queue: String,
    #[serde(rename = "MaxReceiveCount")]
    pub max_receive_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub(crate) struct Attributes {
    pub visibility_timeout: u32,
    pub delay_seconds: u32,
    pub retention_seconds: u32,
    pub fifo: bool,
    pub content_based_deduplication: bool,
    #[serde(flatten)]
    pub dead_letter: Option<DeadLetter>,
}

impl Default for Attributes {
    fn default() -> Self {
        Self {
            visibility_timeout: 30,
            delay_seconds: 0,
            retention_seconds: 345_600,
            fifo: false,
            content_based_deduplication: false,
            dead_letter: None,
        }
    }
}

impl Attributes {
    /// Overwrites the attributes named in `patch`; `dead_letter` is the
    /// already paired dead-letter configuration of that patch.
    pub(crate) fn apply(&mut self, patch: &AttributePatch, dead_letter: Option<DeadLetter>) {
        let AttributePatch {
            visibility_timeout,
            delay_seconds,
            retention_seconds,
            fifo,
            content_based_deduplication,
            dead_letter_queue: _,
            max_receive_count: _,
        } = patch;
        if let Some(value) = visibility_timeout {
            self.visibility_timeout = *value;
        }
        if let Some(value) = delay_seconds {
            self.delay_seconds = *value;
        }
        if let Some(value) = retention_seconds {
            self.retention_seconds = *value;
        }
        if let Some(value) = fifo {
            self.fifo = *value;
        }
        if let Some(value) = content_based_deduplication {
            self.content_based_deduplication = *value;
        }
        if dead_letter.is_some() {
            self.dead_letter = dead_letter;
        }
    }
}

/// Position of a message in a FIFO queue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub(crate) struct FifoPosition {
    pub group_id: GroupId,
    pub sequence_number: u64,
}

/// Hidden states carry the instant they end, so a state that has ended reads
/// as visible without any bookkeeping.
#[derive(Debug, Clone, Copy)]
enum Availability {
    Delayed { until: u64 },
    InFlight { handle: ReceiptHandle, until: u64 },
    Visible,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Delayed,
    InFlight,
    Visible,
}

#[derive(Debug)]
pub(crate) struct Message {
    id: MessageId,
    body: String,
    sent_at: u64,
    receive_count: u32,
    first_received_at: Option<u64>,
    fifo: Option<FifoPosition>,
    availability: Availability,
}

impl Message {
    fn state(&self, now: u64) -> State {
        match self.availability {
            Availability::Delayed { until } if now < until => State::Delayed,
            Availability::InFlight { until, .. } if now < until => State::InFlight,
            _ => State::Visible,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub(crate) struct Counts {
    visible: u64,
    in_flight: u64,
    delayed: u64,
}

/// A message as handed to a consumer.
#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub(crate) struct Delivery {
    message_id: MessageId,
    receipt_handle: ReceiptHandle,
    body: String,
    receive_count: u32,
    sent_at: u64,
    first_received_at: u64,
    #[serde(flatten)]
    fifo: Option<FifoPosition>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub(crate) struct Sent {
    message_id: MessageId,
    #[serde(skip_serializing_if = "Option::is_none")]
    sequence_number: Option<u64>,
}

/// Outcome of one `ReceiveMessage` on the queue that was read.
#[derive(Debug, Default)]
pub(crate) struct Received {
    pub deliveries: Vec<Delivery>,
    /// Messages that reached the dead-letter threshold, in arrival order. They
    /// are already gone from this queue and must be handed to the dead-letter
    /// queue with [`Queue::accept_dead_letter`].
    pub dead_lettered: Vec<Message>,
}

#[derive(Debug)]
struct DedupRecord {
    at: u64,
    message_id: MessageId,
    sequence_number: u64,
}

#[derive(Debug, PartialEq, Eq)]
enum Step {
    Skip,
    DeadLetter,
    Deliver,
}

#[derive(Debug)]
pub(crate) struct Queue {
    attributes: Attributes,
    /// Messages by arrival index, so iteration is arrival order and removal is
    /// logarithmic. Indexes are never reused within a queue.
    messages: BTreeMap<u64, Message>,
    next_arrival: u64,
    /// `(sent_at, arrival)` of every message, to find the ones past retention
    /// without scanning. Arrival order is not `sent_at` order because
    /// dead-lettered messages keep their original `sent_at`.
    by_age: BTreeSet<(u64, u64)>,
    /// Latest receipt handle of each in-flight message, as an arrival index.
    /// Holds exactly the messages whose availability is `InFlight`.
    handles: HashMap<ReceiptHandle, u64>,
    /// Deduplication ids of FIFO sends. They outlive their messages.
    deduplication: HashMap<String, DedupRecord>,
    next_sequence_number: u64,
}

impl Queue {
    pub(crate) fn new(attributes: Attributes) -> Self {
        Self {
            attributes,
            messages: BTreeMap::new(),
            next_arrival: 0,
            by_age: BTreeSet::new(),
            handles: HashMap::new(),
            deduplication: HashMap::new(),
            next_sequence_number: 1,
        }
    }

    pub(crate) fn attributes(&self) -> &Attributes {
        &self.attributes
    }

    /// Applies a validated patch. Lowering retention removes messages now.
    pub(crate) fn set_attributes(
        &mut self,
        patch: &AttributePatch,
        dead_letter: Option<DeadLetter>,
        now: u64,
    ) {
        self.attributes.apply(patch, dead_letter);
        self.expire(now);
    }

    pub(crate) fn clear_dead_letter_to(&mut self, deleted_queue: &str) {
        if self
            .attributes
            .dead_letter
            .as_ref()
            .is_some_and(|dead_letter| dead_letter.queue == deleted_queue)
        {
            self.attributes.dead_letter = None;
        }
    }

    pub(crate) fn counts(&self, now: u64) -> Counts {
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

    pub(crate) fn purge(&mut self) {
        self.messages.clear();
        self.by_age.clear();
        self.handles.clear();
    }

    /// Removes every message whose retention has run out.
    pub(crate) fn expire(&mut self, now: u64) {
        let retention = u64::from(self.attributes.retention_seconds);
        while let Some(&(sent_at, arrival)) = self.by_age.first() {
            if now < sent_at + retention {
                break;
            }
            self.remove(arrival);
        }
    }

    fn insert(&mut self, message: Message) {
        let arrival = self.next_arrival;
        self.next_arrival += 1;
        self.by_age.insert((message.sent_at, arrival));
        self.messages.insert(arrival, message);
    }

    fn remove(&mut self, arrival: u64) -> Message {
        let message = self
            .messages
            .remove(&arrival)
            .expect("callers only pass arrival indexes of messages in the queue");
        self.by_age.remove(&(message.sent_at, arrival));
        if let Availability::InFlight { handle, .. } = message.availability {
            self.handles.remove(&handle);
        }
        message
    }

    fn take_sequence_number(&mut self) -> u64 {
        let number = self.next_sequence_number;
        self.next_sequence_number += 1;
        number
    }

    pub(crate) fn send(
        &mut self,
        message: NewMessage,
        now: u64,
        ids: &mut IdSource,
    ) -> Result<Sent, Error> {
        let NewMessage {
            body,
            delay_seconds,
            group_id,
            deduplication_id,
        } = message;
        let unsupported = |what: &str| Error::new(ErrorCode::UnsupportedOperation, what);

        if !self.attributes.fifo {
            if group_id.is_some() || deduplication_id.is_some() {
                return Err(unsupported("GroupId and DeduplicationId need a FIFO queue"));
            }
        } else if delay_seconds.is_some() {
            return Err(unsupported(
                "DelaySeconds per message is not supported on a FIFO queue",
            ));
        }

        let fifo_identity = if self.attributes.fifo {
            let group_id = group_id.ok_or_else(|| {
                Error::new(
                    ErrorCode::MissingParameter,
                    "GroupId is required on a FIFO queue",
                )
            })?;
            let key = match deduplication_id {
                Some(id) => id,
                None if self.attributes.content_based_deduplication => body.clone(),
                None => {
                    return Err(Error::new(
                        ErrorCode::MissingParameter,
                        "DeduplicationId is required unless the queue deduplicates by content",
                    ));
                }
            };
            if let Some(original) = self
                .deduplication
                .get(&key)
                .filter(|record| now - record.at < DEDUPLICATION_WINDOW)
            {
                return Ok(Sent {
                    message_id: original.message_id,
                    sequence_number: Some(original.sequence_number),
                });
            }
            Some((group_id, key))
        } else {
            None
        };

        if self.messages.len() >= MAX_MESSAGES_PER_QUEUE {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                "the queue already holds the maximum number of messages",
            ));
        }

        let message_id = ids.next_message();
        let fifo = fifo_identity.map(|(group_id, key)| {
            let sequence_number = self.take_sequence_number();
            self.deduplication.insert(
                key,
                DedupRecord {
                    at: now,
                    message_id,
                    sequence_number,
                },
            );
            FifoPosition {
                group_id,
                sequence_number,
            }
        });
        let delay = delay_seconds.unwrap_or(self.attributes.delay_seconds);
        let sequence_number = fifo.as_ref().map(|position| position.sequence_number);
        self.insert(Message {
            id: message_id,
            body,
            sent_at: now,
            receive_count: 0,
            first_received_at: None,
            fifo,
            availability: Availability::Delayed {
                until: now + u64::from(delay),
            },
        });
        Ok(Sent {
            message_id,
            sequence_number,
        })
    }

    /// Appends a message moved here from another queue's dead-letter
    /// configuration. The move skips deduplication and the message limit.
    pub(crate) fn accept_dead_letter(&mut self, message: Message, now: u64) {
        let fifo = message.fifo.map(|position| FifoPosition {
            group_id: position.group_id,
            sequence_number: self.take_sequence_number(),
        });
        let message = Message {
            receive_count: 0,
            first_received_at: None,
            fifo,
            availability: Availability::Visible,
            ..message
        };
        let retention = u64::from(self.attributes.retention_seconds);
        if now < message.sent_at + retention {
            self.insert(message);
        }
    }

    pub(crate) fn receive(
        &mut self,
        max_messages: u32,
        visibility_timeout: u32,
        now: u64,
        ids: &mut IdSource,
    ) -> Received {
        let mut received = Received::default();
        // Groups that still have an earlier message in the queue. A message
        // moved to the dead-letter queue leaves, so it does not block its group.
        let mut blocked_groups: HashSet<GroupId> = HashSet::new();
        let mut next = 0;

        while received.deliveries.len() < max_messages as usize {
            let Some((&arrival, message)) = self.messages.range(next..).next() else {
                break;
            };
            next = arrival + 1;

            let group = message.fifo.as_ref().map(|position| &position.group_id);
            let eligible = message.state(now) == State::Visible
                && group.is_none_or(|group| !blocked_groups.contains(group));
            let step = if !eligible {
                Step::Skip
            } else if self
                .attributes
                .dead_letter
                .as_ref()
                .is_some_and(|dead_letter| message.receive_count >= dead_letter.max_receive_count)
            {
                Step::DeadLetter
            } else {
                Step::Deliver
            };
            if step != Step::DeadLetter
                && let Some(group) = group.filter(|group| !blocked_groups.contains(*group))
            {
                blocked_groups.insert(group.clone());
            }

            match step {
                Step::Skip => {}
                Step::DeadLetter => received.dead_lettered.push(self.remove(arrival)),
                Step::Deliver => {
                    let handle = ids.next_handle();
                    received.deliveries.push(self.deliver(
                        arrival,
                        handle,
                        visibility_timeout,
                        now,
                    ));
                }
            }
        }
        received
    }

    fn deliver(
        &mut self,
        arrival: u64,
        handle: ReceiptHandle,
        visibility_timeout: u32,
        now: u64,
    ) -> Delivery {
        let message = self
            .messages
            .get_mut(&arrival)
            .expect("callers only pass arrival indexes of messages in the queue");
        if let Availability::InFlight {
            handle: previous, ..
        } = message.availability
        {
            self.handles.remove(&previous);
        }
        message.receive_count += 1;
        let first_received_at = *message.first_received_at.get_or_insert(now);
        message.availability = Availability::InFlight {
            handle,
            until: now + u64::from(visibility_timeout),
        };
        self.handles.insert(handle, arrival);
        Delivery {
            message_id: message.id,
            receipt_handle: handle,
            body: message.body.clone(),
            receive_count: message.receive_count,
            sent_at: message.sent_at,
            first_received_at,
            fifo: message.fifo.clone(),
        }
    }

    /// Arrival index of the message that `handle` is the latest handle of, if
    /// that message is in flight right now.
    fn in_flight(&self, handle: ReceiptHandle, now: u64) -> Result<u64, Error> {
        let expired = || {
            Error::new(
                ErrorCode::ReceiptHandleExpired,
                "the receipt handle no longer refers to an in-flight message of this queue",
            )
        };
        let arrival = *self.handles.get(&handle).ok_or_else(expired)?;
        let message = self
            .messages
            .get(&arrival)
            .expect("handles only point at messages in the queue");
        if message.state(now) == State::InFlight {
            Ok(arrival)
        } else {
            Err(expired())
        }
    }

    pub(crate) fn delete_message(&mut self, handle: ReceiptHandle, now: u64) -> Result<(), Error> {
        let arrival = self.in_flight(handle, now)?;
        self.remove(arrival);
        Ok(())
    }

    pub(crate) fn change_visibility(
        &mut self,
        handle: ReceiptHandle,
        visibility_timeout: u32,
        now: u64,
    ) -> Result<(), Error> {
        let arrival = self.in_flight(handle, now)?;
        let message = self
            .messages
            .get_mut(&arrival)
            .expect("in_flight returns the index of a message in the queue");
        message.availability = Availability::InFlight {
            handle,
            until: now + u64::from(visibility_timeout),
        };
        Ok(())
    }
}
