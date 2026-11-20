//! A single queue: its messages, their states and the rules that act on one queue alone.

use std::collections::{BTreeMap, HashMap, HashSet};

use serde::Serialize;

use crate::attributes::Attributes;
use crate::error::{Error, ErrorCode};
use crate::types::{Body, GroupId, Handles, MessageId, ReceiptHandle, Seconds, SequenceNumber};

pub const MAX_MESSAGES: usize = 100_000;
const DEDUPLICATION_WINDOW: Seconds = Seconds(300);

/// Position in a queue's arrival order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Arrival(u64);

/// What only FIFO messages carry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct FifoInfo {
    pub group_id: GroupId,
    pub sequence_number: SequenceNumber,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Availability {
    Visible,
    Delayed {
        until: Seconds,
    },
    /// The latest delivery. Once `until` has passed the message is visible again
    /// and `handle` is stale.
    InFlight {
        until: Seconds,
        handle: ReceiptHandle,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Visible,
    InFlight,
    Delayed,
}

#[derive(Debug, Clone)]
pub struct Message {
    id: MessageId,
    body: Body,
    sent_at: Seconds,
    receive_count: u32,
    first_received_at: Option<Seconds>,
    fifo: Option<FifoInfo>,
    availability: Availability,
}

impl Message {
    fn state(&self, now: Seconds) -> State {
        match self.availability {
            Availability::Delayed { until } if now < until => State::Delayed,
            Availability::InFlight { until, .. } if now < until => State::InFlight,
            _ => State::Visible,
        }
    }
}

/// The number of messages in each state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Counts {
    pub visible: usize,
    pub in_flight: usize,
    pub delayed: usize,
}

/// A message as a receiver sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Delivered {
    pub message_id: MessageId,
    pub receipt_handle: ReceiptHandle,
    pub body: Body,
    pub receive_count: u32,
    pub sent_at: Seconds,
    pub first_received_at: Seconds,
    #[serde(flatten)]
    pub fifo: Option<FifoInfo>,
}

/// The identity a send reports back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sent {
    pub id: MessageId,
    pub sequence: Option<SequenceNumber>,
}

pub enum Admission {
    Accepted(Sent),
    /// A FIFO send that repeats a deduplication id still in its window.
    Duplicate(Sent),
}

/// The caller's choices for one send, already checked for shape.
pub struct Outgoing {
    pub body: Body,
    pub delay: Option<Seconds>,
    pub group: Option<GroupId>,
    pub deduplication_id: Option<String>,
}

/// What one receive call did to a queue.
pub struct Received {
    pub delivered: Vec<Delivered>,
    /// Messages that left this queue for its dead-letter queue, in order.
    pub dead_letters: Vec<Message>,
}

#[derive(Debug)]
struct DedupEntry {
    accepted_at: Seconds,
    sent: Sent,
}

enum Step {
    Deliver(Arrival),
    DeadLetter(Arrival),
}

#[derive(Debug)]
pub struct Queue {
    attributes: Attributes,
    messages: BTreeMap<Arrival, Message>,
    next_arrival: u64,
    next_sequence: u64,
    /// The latest handle of each message that has been delivered.
    by_handle: HashMap<ReceiptHandle, Arrival>,
    deduplication: HashMap<String, DedupEntry>,
    /// The clock value at which expired messages were last removed.
    swept_at: Seconds,
}

impl Queue {
    pub fn new(attributes: Attributes, now: Seconds) -> Self {
        Self {
            attributes,
            messages: BTreeMap::new(),
            next_arrival: 0,
            next_sequence: 0,
            by_handle: HashMap::new(),
            deduplication: HashMap::new(),
            swept_at: now,
        }
    }

    pub fn attributes(&self) -> &Attributes {
        &self.attributes
    }

    pub fn attributes_mut(&mut self) -> &mut Attributes {
        &mut self.attributes
    }

    /// Removes the messages that are expired at `now`. Retention is the same for
    /// every message of the queue, so nothing can expire between clock changes.
    pub fn refresh(&mut self, now: Seconds) {
        if self.swept_at != now {
            self.expire(now);
        }
    }

    /// Removes expired messages unconditionally; needed after retention changes.
    pub fn expire(&mut self, now: Seconds) {
        self.swept_at = now;
        let retention = self.attributes.retention_seconds;
        let by_handle = &mut self.by_handle;
        self.messages.retain(|_, message| {
            let expired = now >= message.sent_at + retention;
            if expired && let Availability::InFlight { handle, .. } = message.availability {
                by_handle.remove(&handle);
            }
            !expired
        });
    }

    pub fn counts(&self, now: Seconds) -> Counts {
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

    pub fn purge(&mut self) {
        self.messages.clear();
        self.by_handle.clear();
    }

    fn next_sequence(&mut self) -> SequenceNumber {
        self.next_sequence += 1;
        SequenceNumber(self.next_sequence)
    }

    fn push(&mut self, message: Message) {
        let arrival = Arrival(self.next_arrival);
        self.next_arrival += 1;
        self.messages.insert(arrival, message);
    }

    fn take(&mut self, arrival: Arrival) -> Option<Message> {
        let message = self.messages.remove(&arrival)?;
        if let Availability::InFlight { handle, .. } = message.availability {
            self.by_handle.remove(&handle);
        }
        Some(message)
    }

    /// Applies the checks and effects of `SendMessage` that depend on this queue.
    /// `id` is used only if the message is accepted.
    pub fn send(
        &mut self,
        now: Seconds,
        id: MessageId,
        outgoing: Outgoing,
    ) -> Result<Admission, Error> {
        let Outgoing {
            body,
            delay,
            group,
            deduplication_id,
        } = outgoing;
        let fifo_send = if self.attributes.fifo {
            if delay.is_some() {
                return Err(unsupported("DelaySeconds is not supported on a FIFO queue"));
            }
            let group = group.ok_or_else(|| Error::missing("GroupId"))?;
            let key = match deduplication_id {
                Some(key) => key,
                None if self.attributes.content_based_deduplication => body.as_str().to_owned(),
                None => return Err(Error::missing("DeduplicationId")),
            };
            Some((group, key))
        } else {
            if group.is_some() || deduplication_id.is_some() {
                return Err(unsupported(
                    "GroupId and DeduplicationId are only supported on a FIFO queue",
                ));
            }
            None
        };

        if let Some((_, key)) = &fifo_send
            && let Some(entry) = self.deduplication.get(key)
            && now < entry.accepted_at + DEDUPLICATION_WINDOW
        {
            return Ok(Admission::Duplicate(entry.sent));
        }
        if self.messages.len() >= MAX_MESSAGES {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                format!("a queue holds at most {MAX_MESSAGES} messages"),
            ));
        }

        let delay = delay.unwrap_or(self.attributes.delay_seconds);
        let (fifo, sequence) = match fifo_send {
            Some((group_id, key)) => {
                let sequence_number = self.next_sequence();
                let sent = Sent {
                    id,
                    sequence: Some(sequence_number),
                };
                self.deduplication.insert(
                    key,
                    DedupEntry {
                        accepted_at: now,
                        sent,
                    },
                );
                (
                    Some(FifoInfo {
                        group_id,
                        sequence_number,
                    }),
                    Some(sequence_number),
                )
            }
            None => (None, None),
        };
        self.push(Message {
            id,
            body,
            sent_at: now,
            receive_count: 0,
            first_received_at: None,
            fifo,
            availability: Availability::Delayed { until: now + delay },
        });
        Ok(Admission::Accepted(Sent { id, sequence }))
    }

    /// Delivers up to `max` messages, moving those that exceeded the receive limit
    /// out of the queue.
    pub fn receive(
        &mut self,
        now: Seconds,
        max: u32,
        visibility_timeout: Option<Seconds>,
        handles: &mut Handles,
    ) -> Received {
        let visibility_timeout = visibility_timeout.unwrap_or(self.attributes.visibility_timeout);
        let mut delivered = Vec::new();
        let mut dead_letters = Vec::new();
        for step in self.plan_receive(now, max) {
            match step {
                Step::Deliver(arrival) => {
                    delivered.extend(self.deliver(arrival, now, visibility_timeout, handles))
                }
                Step::DeadLetter(arrival) => dead_letters.extend(self.take(arrival)),
            }
        }
        Received {
            delivered,
            dead_letters,
        }
    }

    /// Examines the messages in arrival order. Delivering a message does not change
    /// whether a later one is eligible, except through its group, so the decisions
    /// can all be made before any of them is applied.
    fn plan_receive(&self, now: Seconds, max: u32) -> Vec<Step> {
        let mut steps = Vec::new();
        let mut delivering = 0;
        // Groups that still have an earlier message in the queue.
        let mut occupied: HashSet<&GroupId> = HashSet::new();
        for (&arrival, message) in &self.messages {
            if delivering == max {
                break;
            }
            let group = message.fifo.as_ref().map(|fifo| &fifo.group_id);
            let blocked = group.is_some_and(|group| occupied.contains(group));
            if blocked || message.state(now) != State::Visible {
                occupied.extend(group);
                continue;
            }
            match &self.attributes.dead_letter {
                Some(dead_letter) if message.receive_count >= dead_letter.max_receive_count => {
                    steps.push(Step::DeadLetter(arrival));
                }
                _ => {
                    steps.push(Step::Deliver(arrival));
                    delivering += 1;
                    occupied.extend(group);
                }
            }
        }
        steps
    }

    fn deliver(
        &mut self,
        arrival: Arrival,
        now: Seconds,
        visibility_timeout: Seconds,
        handles: &mut Handles,
    ) -> Option<Delivered> {
        let message = self.messages.get_mut(&arrival)?;
        if let Availability::InFlight { handle, .. } = message.availability {
            self.by_handle.remove(&handle);
        }
        let handle = handles.issue();
        self.by_handle.insert(handle, arrival);
        message.receive_count += 1;
        let first_received_at = *message.first_received_at.get_or_insert(now);
        message.availability = Availability::InFlight {
            until: now + visibility_timeout,
            handle,
        };
        Some(Delivered {
            message_id: message.id,
            receipt_handle: handle,
            body: message.body.clone(),
            receive_count: message.receive_count,
            sent_at: message.sent_at,
            first_received_at,
            fifo: message.fifo.clone(),
        })
    }

    /// Adds a message that another queue gave up on to the end of this one.
    pub fn accept_dead_letter(&mut self, mut message: Message, now: Seconds) {
        message.receive_count = 0;
        message.first_received_at = None;
        message.availability = Availability::Visible;
        if let Some(fifo) = &mut message.fifo {
            fifo.sequence_number = self.next_sequence();
        }
        if now < message.sent_at + self.attributes.retention_seconds {
            self.push(message);
        }
    }

    fn in_flight(&self, handle: ReceiptHandle, now: Seconds) -> Result<Arrival, Error> {
        self.by_handle
            .get(&handle)
            .copied()
            .filter(|arrival| {
                self.messages
                    .get(arrival)
                    .is_some_and(|message| message.state(now) == State::InFlight)
            })
            .ok_or_else(|| {
                Error::new(
                    ErrorCode::ReceiptHandleExpired,
                    format!("receipt handle {handle} is not usable in this queue"),
                )
            })
    }

    pub fn delete(&mut self, handle: ReceiptHandle, now: Seconds) -> Result<(), Error> {
        let arrival = self.in_flight(handle, now)?;
        self.take(arrival);
        Ok(())
    }

    pub fn change_visibility(
        &mut self,
        handle: ReceiptHandle,
        now: Seconds,
        timeout: Seconds,
    ) -> Result<(), Error> {
        let arrival = self.in_flight(handle, now)?;
        if let Some(message) = self.messages.get_mut(&arrival) {
            message.availability = Availability::InFlight {
                until: now + timeout,
                handle,
            };
        }
        Ok(())
    }
}

fn unsupported(message: &str) -> Error {
    Error::new(ErrorCode::UnsupportedOperation, message)
}
