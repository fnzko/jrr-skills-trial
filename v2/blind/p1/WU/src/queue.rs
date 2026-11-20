//! One queue: its attributes and messages, and everything that only needs
//! this queue's own state.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use crate::attributes::{AttributePatch, Attributes, DeadLetter};
use crate::error::{Error, ErrorCode};
use crate::ids::{MessageId, ReceiptHandle};

const MESSAGE_LIMIT: usize = 100_000;

/// A FIFO send with the same deduplication id as an accepted one is dropped
/// for this many seconds after that one was accepted.
const DEDUPLICATION_WINDOW: u64 = 300;

/// Position of a message in its queue's arrival order.
type Arrival = u64;

/// Where a message of a FIFO queue belongs.
#[derive(Debug, Clone)]
pub struct FifoPlace {
    pub group: String,
    pub sequence_number: u64,
}

/// The reason a message is not visible.
#[derive(Debug, Clone, Copy)]
pub enum Hold {
    /// Sent with a delay; visible from `until` on.
    Delayed { until: u64 },
    /// Delivered; visible again from `until` on.
    InFlight { until: u64, handle: ReceiptHandle },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Visible,
    InFlight,
    Delayed,
}

#[derive(Debug, Clone)]
pub struct Message {
    pub id: MessageId,
    pub body: String,
    pub sent_at: u64,
    pub receive_count: u64,
    pub first_received_at: Option<u64>,
    /// `Some` exactly in FIFO queues.
    pub fifo: Option<FifoPlace>,
    /// `None` for a message that was never held back.
    pub hold: Option<Hold>,
}

impl Message {
    pub fn state(&self, now: u64) -> State {
        match self.hold {
            Some(Hold::Delayed { until }) if now < until => State::Delayed,
            Some(Hold::InFlight { until, .. }) if now < until => State::InFlight,
            _ => State::Visible,
        }
    }
}

/// The number of messages in each state.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub visible: usize,
    pub in_flight: usize,
    pub delayed: usize,
}

/// What a client asks to add to a queue, once its values are checked.
#[derive(Debug)]
pub struct Submission {
    pub body: String,
    pub delay_seconds: Option<u64>,
    pub group: Option<String>,
    pub deduplication_id: Option<String>,
}

/// The outcome of a send that did not fail.
#[derive(Debug, PartialEq, Eq)]
pub struct Sent {
    pub id: MessageId,
    pub sequence_number: Option<u64>,
    /// The send repeated a recent one; `id` and `sequence_number` are those
    /// of the original and the queue is unchanged.
    pub duplicate: bool,
}

/// A message handed to a consumer.
#[derive(Debug)]
pub struct Delivery {
    pub handle: ReceiptHandle,
    /// The message as it was right after the delivery.
    pub message: Message,
}

/// Messages a receive moved out of a queue.
#[derive(Debug)]
pub struct DeadLetters {
    /// The queue they belong in now.
    pub queue: String,
    pub messages: Vec<Message>,
}

#[derive(Debug)]
pub struct Received {
    pub deliveries: Vec<Delivery>,
    pub dead_letters: Option<DeadLetters>,
}

/// The message that a deduplication id was last accepted for.
#[derive(Debug)]
struct Accepted {
    at: u64,
    id: MessageId,
    sequence_number: u64,
}

#[derive(Debug)]
pub struct Queue {
    attributes: Attributes,
    messages: BTreeMap<Arrival, Message>,
    /// `(SentAt, arrival)` of every message, to find expired ones without a
    /// scan: arrival order is not sent order for messages moved in from a
    /// dead-letter source.
    by_sent_at: BTreeSet<(u64, Arrival)>,
    next_arrival: Arrival,
    /// FIFO queues only. Outlives the messages it was recorded for.
    next_sequence_number: u64,
    accepted: HashMap<String, Accepted>,
    /// The message each handle was last issued for, while that message is
    /// still in the queue and has not been delivered again.
    handles: HashMap<ReceiptHandle, Arrival>,
}

impl Queue {
    pub fn new(attributes: Attributes) -> Self {
        Self {
            attributes,
            messages: BTreeMap::new(),
            by_sent_at: BTreeSet::new(),
            next_arrival: 0,
            next_sequence_number: 1,
            accepted: HashMap::new(),
            handles: HashMap::new(),
        }
    }

    pub fn attributes(&self) -> &Attributes {
        &self.attributes
    }

    /// Applies new attributes. Retention changes take effect at once.
    pub fn set_attributes(
        &mut self,
        patch: &AttributePatch,
        dead_letter: Option<DeadLetter>,
        now: u64,
    ) {
        self.attributes.apply(patch, dead_letter);
        self.expire(now);
    }

    pub fn clear_dead_letter(&mut self) {
        self.attributes.dead_letter = None;
    }

    pub fn counts(&self, now: u64) -> Counts {
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

    pub fn purge(&mut self) {
        self.messages.clear();
        self.by_sent_at.clear();
        self.handles.clear();
    }

    /// Removes every message whose retention has run out at `now`.
    ///
    /// Every operation on a queue calls this first, which gives the same
    /// result as removing messages the moment they expire.
    pub fn expire(&mut self, now: u64) {
        let retention = self.attributes.retention_seconds;
        while let Some(&(sent_at, arrival)) = self.by_sent_at.first() {
            if now < sent_at + retention {
                break;
            }
            self.take(arrival);
        }
    }

    /// Adds a message to the end of the queue, unless the send is invalid, a
    /// duplicate, or the queue is full. `id` is used only for a new message.
    pub fn send(&mut self, submission: Submission, id: MessageId, now: u64) -> Result<Sent, Error> {
        let fifo = self.attributes.fifo;
        let unsupported = if fifo {
            submission
                .delay_seconds
                .is_some()
                .then_some("`DelaySeconds` per message")
        } else {
            (submission.group.is_some() || submission.deduplication_id.is_some())
                .then_some("`GroupId` and `DeduplicationId`")
        };
        if let Some(parameter) = unsupported {
            return Err(Error::new(
                ErrorCode::UnsupportedOperation,
                format!("{parameter} not supported by this type of queue"),
            ));
        }
        let place = if fifo {
            let Some(group) = submission.group else {
                return Err(missing("GroupId"));
            };
            let deduplication_id = match &submission.deduplication_id {
                Some(id) => id,
                None if self.attributes.content_based_deduplication => &submission.body,
                None => return Err(missing("DeduplicationId")),
            };
            if let Some(original) = self.accepted.get(deduplication_id)
                && now < original.at + DEDUPLICATION_WINDOW
            {
                return Ok(Sent {
                    id: original.id,
                    sequence_number: Some(original.sequence_number),
                    duplicate: true,
                });
            }
            Some((group, deduplication_id.clone()))
        } else {
            None
        };
        if self.messages.len() >= MESSAGE_LIMIT {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                format!("a queue holds at most {MESSAGE_LIMIT} messages"),
            ));
        }

        let delay = submission
            .delay_seconds
            .unwrap_or(self.attributes.delay_seconds);
        let fifo_place = place.map(|(group, deduplication_id)| {
            let sequence_number = self.next_sequence_number;
            self.next_sequence_number += 1;
            self.accepted.insert(
                deduplication_id,
                Accepted {
                    at: now,
                    id,
                    sequence_number,
                },
            );
            FifoPlace {
                group,
                sequence_number,
            }
        });
        let sequence_number = fifo_place.as_ref().map(|place| place.sequence_number);
        self.push(Message {
            id,
            body: submission.body,
            sent_at: now,
            receive_count: 0,
            first_received_at: None,
            fifo: fifo_place,
            hold: (delay > 0).then_some(Hold::Delayed { until: now + delay }),
        });
        Ok(Sent {
            id,
            sequence_number,
            duplicate: false,
        })
    }

    /// Delivers up to `max_messages` eligible messages in arrival order.
    ///
    /// Each delivery takes a handle number from `next_handle`. Messages that
    /// reached the dead-letter limit are taken out instead and returned for
    /// the caller to place in the dead-letter queue.
    pub fn receive(
        &mut self,
        max_messages: u64,
        visibility_timeout: Option<u64>,
        now: u64,
        next_handle: &mut u64,
    ) -> Received {
        let visibility_timeout = visibility_timeout.unwrap_or(self.attributes.visibility_timeout);
        let dead_letter = self.attributes.dead_letter.clone();
        // Groups with an earlier message that stays in the queue.
        let mut blocked_groups: HashSet<String> = HashSet::new();
        let mut remaining = max_messages;
        let mut deliveries = Vec::new();
        let mut dead = Vec::new();

        for (&arrival, message) in &mut self.messages {
            if remaining == 0 {
                break;
            }
            if let Some(place) = &message.fifo
                && blocked_groups.contains(&place.group)
            {
                continue;
            }
            if message.state(now) == State::Visible {
                if dead_letter
                    .as_ref()
                    .is_some_and(|d| message.receive_count >= d.max_receive_count)
                {
                    dead.push(arrival);
                    continue;
                }
                message.receive_count += 1;
                message.first_received_at.get_or_insert(now);
                if let Some(Hold::InFlight { handle, .. }) = message.hold {
                    self.handles.remove(&handle);
                }
                let handle = ReceiptHandle::new(*next_handle);
                *next_handle += 1;
                message.hold = Some(Hold::InFlight {
                    until: now + visibility_timeout,
                    handle,
                });
                self.handles.insert(handle, arrival);
                remaining -= 1;
                deliveries.push(Delivery {
                    handle,
                    message: message.clone(),
                });
            }
            if let Some(place) = &message.fifo {
                blocked_groups.insert(place.group.clone());
            }
        }

        let dead_letters =
            dead_letter
                .filter(|_| !dead.is_empty())
                .map(|dead_letter| DeadLetters {
                    queue: dead_letter.queue,
                    messages: dead.iter().map(|&arrival| self.take(arrival)).collect(),
                });
        Received {
            deliveries,
            dead_letters,
        }
    }

    /// Removes the message that `handle` was issued for.
    pub fn delete(&mut self, handle: ReceiptHandle, now: u64) -> Result<(), Error> {
        let arrival = self.in_flight(handle, now)?;
        self.take(arrival);
        Ok(())
    }

    /// Makes the message that `handle` was issued for in flight until
    /// `now + timeout`.
    pub fn change_visibility(
        &mut self,
        handle: ReceiptHandle,
        timeout: u64,
        now: u64,
    ) -> Result<(), Error> {
        let arrival = self.in_flight(handle, now)?;
        let message = self
            .messages
            .get_mut(&arrival)
            .expect("the handle index only points at stored messages");
        message.hold = Some(Hold::InFlight {
            until: now + timeout,
            handle,
        });
        Ok(())
    }

    /// The message whose latest handle is `handle`, if it is in flight.
    fn in_flight(&self, handle: ReceiptHandle, now: u64) -> Result<Arrival, Error> {
        self.handles
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
                    format!("receipt handle {handle} is not the handle of an in-flight message of this queue"),
                )
            })
    }

    /// Adds messages moved from another queue after too many deliveries.
    ///
    /// They keep their id, body, `SentAt` and group, start over as never
    /// received and visible, and are not recorded for deduplication.
    pub fn adopt(&mut self, messages: Vec<Message>, now: u64) {
        for message in messages {
            let fifo = message.fifo.map(|place| {
                let sequence_number = self.next_sequence_number;
                self.next_sequence_number += 1;
                FifoPlace {
                    group: place.group,
                    sequence_number,
                }
            });
            self.push(Message {
                receive_count: 0,
                first_received_at: None,
                fifo,
                hold: None,
                ..message
            });
        }
        // The messages keep counting retention from their original `SentAt`.
        self.expire(now);
    }

    fn push(&mut self, message: Message) {
        let arrival = self.next_arrival;
        self.next_arrival += 1;
        self.by_sent_at.insert((message.sent_at, arrival));
        self.messages.insert(arrival, message);
    }

    fn take(&mut self, arrival: Arrival) -> Message {
        let message = self
            .messages
            .remove(&arrival)
            .expect("arrival keys come from this queue's own index");
        self.by_sent_at.remove(&(message.sent_at, arrival));
        if let Some(Hold::InFlight { handle, .. }) = message.hold {
            self.handles.remove(&handle);
        }
        message
    }
}

fn missing(name: &str) -> Error {
    Error::new(
        ErrorCode::MissingParameter,
        format!("`{name}` is required by a FIFO queue"),
    )
}
