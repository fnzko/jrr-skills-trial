//! One queue: its attributes, its messages and its deduplication memory.

use std::collections::{BTreeMap, HashMap, HashSet};

use serde::Serialize;

use crate::attributes::{AttributeUpdates, Attributes, DeadLetter, Kind};
use crate::error::{Error, ErrorCode};
use crate::ids::{Counters, GroupId, MessageId, QueueName, ReceiptHandle, SequenceNumber};
use crate::time::{Seconds, Time};

const MAX_MESSAGES: usize = 100_000;
const DEDUPLICATION_WINDOW: Seconds = Seconds(300);

/// Arrival position of a message in its queue; never reused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct Slot(u64);

/// FIFO-only message data.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct FifoTag {
    group_id: GroupId,
    sequence_number: SequenceNumber,
}

/// What keeps a message from being received right now, if anything.
#[derive(Debug, Clone, Copy)]
enum Hold {
    /// Nothing holds the message back; used for messages moved in from a
    /// dead-letter source.
    Free,
    /// Delayed until `until`.
    Delayed { until: Time },
    /// In flight until `until`, as long as `handle` is its latest handle.
    InFlight { until: Time, handle: ReceiptHandle },
}

/// The three observable states of a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Visible,
    InFlight,
    Delayed,
}

/// A message held by a queue.
#[derive(Debug)]
pub struct Message {
    id: MessageId,
    body: String,
    sent_at: Time,
    receive_count: u32,
    first_received_at: Option<Time>,
    fifo: Option<FifoTag>,
    hold: Hold,
}

impl Message {
    fn state(&self, now: Time) -> State {
        match self.hold {
            Hold::Delayed { until } if now < until => State::Delayed,
            Hold::InFlight { until, .. } if now < until => State::InFlight,
            _ => State::Visible,
        }
    }
}

/// How many messages are in each state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Counts {
    visible: usize,
    in_flight: usize,
    delayed: usize,
}

/// The parameters of a `SendMessage` that concern the message itself.
#[derive(Debug)]
pub struct MessageDraft {
    pub body: String,
    pub delay: Option<Seconds>,
    pub group_id: Option<GroupId>,
    pub deduplication_id: Option<String>,
}

/// The result of an accepted (or duplicate) send.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Sent {
    message_id: MessageId,
    #[serde(skip_serializing_if = "Option::is_none")]
    sequence_number: Option<SequenceNumber>,
}

/// One message handed to a receiver.
#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Delivery {
    message_id: MessageId,
    receipt_handle: ReceiptHandle,
    body: String,
    receive_count: u32,
    sent_at: Time,
    first_received_at: Time,
    #[serde(flatten)]
    fifo: Option<FifoTag>,
}

/// Messages a receive took out of a queue for its dead-letter queue.
#[derive(Debug)]
pub struct DeadLetters {
    pub queue: QueueName,
    pub messages: Vec<Message>,
}

/// What one `ReceiveMessage` did.
#[derive(Debug)]
pub struct Received {
    pub delivered: Vec<Delivery>,
    pub dead_letters: Option<DeadLetters>,
}

/// A send that was accepted inside the deduplication window.
#[derive(Debug, Clone, Copy)]
struct Accepted {
    until: Time,
    sent: Sent,
}

enum Step {
    Deliver(Slot),
    DeadLetter(Slot),
}

fn unsupported(message: &str) -> Error {
    Error::new(ErrorCode::UnsupportedOperation, message)
}

fn expired_handle(handle: ReceiptHandle) -> Error {
    Error::new(
        ErrorCode::ReceiptHandleExpired,
        format!("receipt handle {handle} is not the current handle of an in-flight message"),
    )
}

/// A queue and everything that belongs to it.
#[derive(Debug)]
pub struct Queue {
    attributes: Attributes,
    messages: BTreeMap<Slot, Message>,
    next_slot: u64,
    next_sequence: u64,
    /// The latest handle of each delivered message that is still held.
    handles: HashMap<ReceiptHandle, Slot>,
    /// Deduplication ids of FIFO sends; they outlive the messages.
    deduplication: HashMap<String, Accepted>,
}

impl Queue {
    /// An empty queue.
    pub fn new(attributes: Attributes) -> Self {
        Self {
            attributes,
            messages: BTreeMap::new(),
            next_slot: 0,
            next_sequence: 1,
            handles: HashMap::new(),
            deduplication: HashMap::new(),
        }
    }

    /// The queue's settings.
    pub fn attributes(&self) -> &Attributes {
        &self.attributes
    }

    /// Applies new attributes and removes messages that are now too old.
    pub fn set_attributes(
        &mut self,
        updates: &AttributeUpdates,
        dead_letter: Option<DeadLetter>,
        now: Time,
    ) {
        self.attributes.apply(updates, dead_letter);
        self.expire(now);
    }

    /// Drops the dead-letter configuration if it points at `deleted`.
    pub fn forget_dead_letter_queue(&mut self, deleted: &str) {
        if self
            .attributes
            .dead_letter
            .as_ref()
            .is_some_and(|dead_letter| dead_letter.queue.as_str() == deleted)
        {
            self.attributes.dead_letter = None;
        }
    }

    /// Removes every message that has outlived the retention period and
    /// forgets deduplication ids whose window has closed.
    pub fn expire(&mut self, now: Time) {
        let retention = self.attributes.retention_seconds;
        let before = self.messages.len();
        self.messages
            .retain(|_, message| now < message.sent_at.plus(retention));
        if self.messages.len() != before {
            let messages = &self.messages;
            self.handles.retain(|_, slot| messages.contains_key(slot));
        }
        self.deduplication
            .retain(|_, accepted| now < accepted.until);
    }

    /// Removes every message in every state.
    pub fn purge(&mut self) {
        self.messages.clear();
        self.handles.clear();
    }

    /// Number of messages in each state.
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

    fn take_sequence(&mut self) -> SequenceNumber {
        let sequence = SequenceNumber(self.next_sequence);
        self.next_sequence += 1;
        sequence
    }

    fn push(&mut self, message: Message) {
        self.messages.insert(Slot(self.next_slot), message);
        self.next_slot += 1;
    }

    /// Removes a message and its receipt handle.
    fn take(&mut self, slot: Slot) -> Message {
        let message = self
            .messages
            .remove(&slot)
            .expect("slots come from this queue's map in the same command");
        if let Hold::InFlight { handle, .. } = message.hold {
            self.handles.remove(&handle);
        }
        message
    }

    /// Runs the `SendMessage` rules from the queue's point of view.
    pub fn send(
        &mut self,
        draft: MessageDraft,
        now: Time,
        counters: &mut Counters,
    ) -> Result<Sent, Error> {
        let MessageDraft {
            body,
            delay,
            group_id,
            deduplication_id,
        } = draft;
        let fifo_send = match self.attributes.kind {
            Kind::Standard => {
                if group_id.is_some() || deduplication_id.is_some() {
                    return Err(unsupported("GroupId and DeduplicationId need a FIFO queue"));
                }
                None
            }
            Kind::Fifo {
                content_based_deduplication,
            } => {
                if delay.is_some() {
                    return Err(unsupported("DelaySeconds is not supported on a FIFO queue"));
                }
                let group_id = group_id.ok_or_else(|| Error::missing("GroupId"))?;
                let key = match deduplication_id {
                    Some(id) => id,
                    None if content_based_deduplication => body.clone(),
                    None => return Err(Error::missing("DeduplicationId")),
                };
                if let Some(earlier) = self.deduplication.get(&key)
                    && now < earlier.until
                {
                    return Ok(earlier.sent);
                }
                Some((group_id, key))
            }
        };
        if self.messages.len() >= MAX_MESSAGES {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                format!("a queue holds at most {MAX_MESSAGES} messages"),
            ));
        }
        let id = counters.message_id();
        let delay = delay.unwrap_or(self.attributes.delay_seconds);
        let mut sent = Sent {
            message_id: id,
            sequence_number: None,
        };
        let fifo = fifo_send.map(|(group_id, key)| {
            let sequence_number = self.take_sequence();
            sent.sequence_number = Some(sequence_number);
            self.deduplication.insert(
                key,
                Accepted {
                    until: now.plus(DEDUPLICATION_WINDOW),
                    sent,
                },
            );
            FifoTag {
                group_id,
                sequence_number,
            }
        });
        self.push(Message {
            id,
            body,
            sent_at: now,
            receive_count: 0,
            first_received_at: None,
            fifo,
            hold: Hold::Delayed {
                until: now.plus(delay),
            },
        });
        Ok(sent)
    }

    /// Delivers up to `max_messages` eligible messages in arrival order and
    /// takes out those that exceeded the receive limit.
    pub fn receive(
        &mut self,
        max_messages: u32,
        visibility_timeout: Option<Seconds>,
        now: Time,
        counters: &mut Counters,
    ) -> Received {
        let steps = self.plan_receive(max_messages, now);
        let timeout = visibility_timeout.unwrap_or(self.attributes.visibility_timeout);
        let mut delivered = Vec::new();
        let mut dead = Vec::new();
        for step in steps {
            match step {
                Step::Deliver(slot) => delivered.push(self.deliver(slot, timeout, now, counters)),
                Step::DeadLetter(slot) => dead.push(self.take(slot)),
            }
        }
        let dead_letters = self
            .attributes
            .dead_letter
            .as_ref()
            .filter(|_| !dead.is_empty())
            .map(|dead_letter| DeadLetters {
                queue: dead_letter.queue.clone(),
                messages: dead,
            });
        Received {
            delivered,
            dead_letters,
        }
    }

    /// Examines messages once each in arrival order. A message is skipped
    /// unless it is visible and no earlier message of its group stays in the
    /// queue; messages that leave for the dead-letter queue do not block.
    fn plan_receive(&self, max_messages: u32, now: Time) -> Vec<Step> {
        let receive_limit = self
            .attributes
            .dead_letter
            .as_ref()
            .map(|dead_letter| dead_letter.max_receive_count);
        let mut steps = Vec::new();
        let mut deliveries = 0;
        let mut blocked_groups: HashSet<&GroupId> = HashSet::new();
        for (&slot, message) in &self.messages {
            let group = message.fifo.as_ref().map(|tag| &tag.group_id);
            let eligible = message.state(now) == State::Visible
                && group.is_none_or(|group| !blocked_groups.contains(group));
            if eligible {
                if receive_limit.is_some_and(|limit| message.receive_count >= limit) {
                    steps.push(Step::DeadLetter(slot));
                    continue;
                }
                steps.push(Step::Deliver(slot));
                deliveries += 1;
            }
            if let Some(group) = group {
                blocked_groups.insert(group);
            }
            if deliveries == max_messages {
                break;
            }
        }
        steps
    }

    fn deliver(
        &mut self,
        slot: Slot,
        timeout: Seconds,
        now: Time,
        counters: &mut Counters,
    ) -> Delivery {
        let message = self
            .messages
            .get_mut(&slot)
            .expect("slots come from this queue's map in the same command");
        let handle = counters.receipt_handle();
        if let Hold::InFlight { handle: old, .. } = message.hold {
            self.handles.remove(&old);
        }
        self.handles.insert(handle, slot);
        message.receive_count += 1;
        message.hold = Hold::InFlight {
            until: now.plus(timeout),
            handle,
        };
        let first_received_at = *message.first_received_at.get_or_insert(now);
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

    /// Adds messages another queue gave up on, as new arrivals here.
    pub fn accept_dead_letters(&mut self, messages: Vec<Message>, now: Time) {
        for mut message in messages {
            message.receive_count = 0;
            message.first_received_at = None;
            message.hold = Hold::Free;
            message.fifo = message.fifo.take().map(|tag| FifoTag {
                group_id: tag.group_id,
                sequence_number: self.take_sequence(),
            });
            let age_limit = message.sent_at.plus(self.attributes.retention_seconds);
            if now < age_limit {
                self.push(message);
            }
        }
    }

    /// The slot of the in-flight message `handle` is the latest handle of.
    fn in_flight_slot(&self, handle: ReceiptHandle, now: Time) -> Result<Slot, Error> {
        self.handles
            .get(&handle)
            .copied()
            .filter(|slot| {
                self.messages
                    .get(slot)
                    .is_some_and(|message| message.state(now) == State::InFlight)
            })
            .ok_or_else(|| expired_handle(handle))
    }

    /// Removes the message `handle` was issued for.
    pub fn delete_message(&mut self, handle: ReceiptHandle, now: Time) -> Result<(), Error> {
        let slot = self.in_flight_slot(handle, now)?;
        self.take(slot);
        Ok(())
    }

    /// Moves the end of the visibility timeout of the message `handle` was
    /// issued for to `timeout` after now.
    pub fn change_visibility(
        &mut self,
        handle: ReceiptHandle,
        timeout: Seconds,
        now: Time,
    ) -> Result<(), Error> {
        let slot = self.in_flight_slot(handle, now)?;
        let message = self
            .messages
            .get_mut(&slot)
            .expect("in_flight_slot returned a slot of this map");
        message.hold = Hold::InFlight {
            until: now.plus(timeout),
            handle,
        };
        Ok(())
    }
}
