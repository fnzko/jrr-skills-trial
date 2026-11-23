//! One queue: its attributes, its messages in arrival order and the indexes
//! that keep retention, receipt handles and deduplication cheap.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use serde::Serialize;

use crate::attributes::{Attributes, QueueKind};
use crate::command::SendRequest;
use crate::error::{Error, ErrorCode};
use crate::names::{DedupKey, GroupId, MessageId, ReceiptHandle};
use crate::reply::ReceivedMessage;

pub const MESSAGE_LIMIT: usize = 100_000;
const DEDUP_WINDOW: u64 = 300;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Hold {
    Delayed { until: u64 },
    InFlight { until: u64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Visible,
    InFlight,
    Delayed,
}

#[derive(Debug)]
pub struct Message {
    id: MessageId,
    body: String,
    sent_at: u64,
    receive_count: u64,
    first_received_at: Option<u64>,
    group: Option<GroupId>,
    hold: Option<Hold>,
    /// The latest handle issued for this message, kept after the visibility
    /// timeout ends so the handle can be told apart from one never issued.
    last_handle: Option<ReceiptHandle>,
}

impl Message {
    fn state(&self, now: u64) -> State {
        match self.hold {
            Some(Hold::Delayed { until }) if now < until => State::Delayed,
            Some(Hold::InFlight { until }) if now < until => State::InFlight,
            _ => State::Visible,
        }
    }

    /// The message as it arrives in a dead-letter queue.
    fn reset_for_dead_letter(mut self) -> Self {
        self.receive_count = 0;
        self.first_received_at = None;
        self.hold = None;
        self.last_handle = None;
        self
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Counts {
    pub visible: u64,
    pub in_flight: u64,
    pub delayed: u64,
}

#[derive(Debug)]
struct Accepted {
    at: u64,
    id: MessageId,
    sequence: u64,
}

/// What `Queue::send` did.
#[derive(Debug, PartialEq, Eq)]
pub struct Sent {
    pub id: MessageId,
    pub sequence: Option<u64>,
    /// A duplicate send returns the original message and uses up no id.
    pub duplicate: bool,
}

/// What `Queue::receive` did.
#[derive(Debug)]
pub struct Received {
    pub delivered: Vec<ReceivedMessage>,
    pub dead: Option<DeadBatch>,
}

/// Messages removed from a queue that belong in its dead-letter queue.
#[derive(Debug)]
pub struct DeadBatch {
    pub queue: String,
    pub messages: Vec<Message>,
}

#[derive(Debug)]
enum Step {
    Deliver,
    DeadLetter,
}

#[derive(Debug)]
pub struct Queue {
    attributes: Attributes,
    /// Keyed by the sequence number, which is also the arrival order.
    messages: BTreeMap<u64, Message>,
    /// `(sent_at, sequence)` of every message, oldest first, for retention.
    by_age: BTreeSet<(u64, u64)>,
    /// Latest handle of every message that has one, to its sequence number.
    handles: HashMap<ReceiptHandle, u64>,
    next_sequence: u64,
    accepted: HashMap<DedupKey, Accepted>,
}

impl Queue {
    pub fn new(attributes: Attributes) -> Self {
        Self {
            attributes,
            messages: BTreeMap::new(),
            by_age: BTreeSet::new(),
            handles: HashMap::new(),
            next_sequence: 1,
            accepted: HashMap::new(),
        }
    }

    pub fn attributes(&self) -> &Attributes {
        &self.attributes
    }

    /// Replaces the attributes. The caller must have expired messages under
    /// the old retention first, so that raising it brings nothing back.
    pub fn set_attributes(&mut self, attributes: Attributes, now: u64) {
        self.attributes = attributes;
        self.expire(now);
    }

    /// Drops the dead-letter configuration if it points at `deleted`.
    pub fn forget_dead_letter(&mut self, deleted: &str) {
        if self
            .attributes
            .dead_letter
            .as_ref()
            .is_some_and(|dead_letter| dead_letter.queue == deleted)
        {
            self.attributes.dead_letter = None;
        }
    }

    /// Removes every message that has outlived the retention period.
    pub fn expire(&mut self, now: u64) {
        let retention = self.attributes.retention;
        while let Some(&(sent_at, sequence)) = self.by_age.first() {
            if sent_at.saturating_add(retention) > now {
                break;
            }
            self.remove(sequence);
        }
    }

    pub fn counts(&self, now: u64) -> Counts {
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

    fn remove(&mut self, sequence: u64) -> Option<Message> {
        let message = self.messages.remove(&sequence)?;
        self.by_age.remove(&(message.sent_at, sequence));
        if let Some(handle) = message.last_handle {
            self.handles.remove(&handle);
        }
        Some(message)
    }

    /// Adds a message at the end of the queue and returns its sequence number.
    fn append(&mut self, message: Message) -> u64 {
        let sequence = self.next_sequence;
        self.next_sequence += 1;
        self.by_age.insert((message.sent_at, sequence));
        self.messages.insert(sequence, message);
        sequence
    }

    fn sequence_number(&self, sequence: u64) -> Option<u64> {
        self.attributes.is_fifo().then_some(sequence)
    }

    /// Applies rules 1 to 5 of `SendMessage`. `id` is used only if the
    /// message is accepted.
    pub fn send(&mut self, now: u64, id: MessageId, request: SendRequest) -> Result<Sent, Error> {
        let SendRequest {
            body,
            delay,
            group,
            dedup,
        } = request;
        let (group, dedup_key) = match self.attributes.kind {
            QueueKind::Standard => {
                if group.is_some() || dedup.is_some() {
                    return Err(unsupported("GroupId and DeduplicationId need a FIFO queue"));
                }
                (None, None)
            }
            QueueKind::Fifo {
                content_based_deduplication,
            } => {
                if delay.is_some() {
                    return Err(unsupported("DelaySeconds is not supported on a FIFO queue"));
                }
                let group = group.ok_or_else(|| missing("GroupId"))?;
                let key = match dedup {
                    Some(key) => key,
                    None if content_based_deduplication => DedupKey::from_body(&body),
                    None => return Err(missing("DeduplicationId")),
                };
                (Some(group), Some(key))
            }
        };
        if let Some(original) = dedup_key
            .as_ref()
            .and_then(|key| self.accepted.get(key))
            .filter(|original| now - original.at < DEDUP_WINDOW)
        {
            return Ok(Sent {
                id: original.id,
                sequence: Some(original.sequence),
                duplicate: true,
            });
        }
        if self.messages.len() >= MESSAGE_LIMIT {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                "the queue holds the maximum number of messages",
            ));
        }
        let delay = delay.unwrap_or(self.attributes.delay);
        let sequence = self.append(Message {
            id,
            body,
            sent_at: now,
            receive_count: 0,
            first_received_at: None,
            group,
            hold: (delay > 0).then(|| Hold::Delayed {
                until: now.saturating_add(delay),
            }),
            last_handle: None,
        });
        if let Some(key) = dedup_key {
            self.accepted.insert(
                key,
                Accepted {
                    at: now,
                    id,
                    sequence,
                },
            );
        }
        Ok(Sent {
            id,
            sequence: self.sequence_number(sequence),
            duplicate: false,
        })
    }

    /// Adds messages moved from another queue, then applies retention so that
    /// a message already too old here is removed on arrival.
    pub fn accept_dead_letters(&mut self, messages: Vec<Message>, now: u64) {
        for message in messages {
            self.append(message.reset_for_dead_letter());
        }
        self.expire(now);
    }

    pub fn receive(
        &mut self,
        now: u64,
        max_messages: u64,
        visibility_timeout: Option<u64>,
        handles: &mut ReceiptHandle,
    ) -> Received {
        let plan = self.plan_receive(now, max_messages);
        let visibility_timeout = visibility_timeout.unwrap_or(self.attributes.visibility_timeout);
        let mut delivered = Vec::new();
        let mut dead = Vec::new();
        for (sequence, step) in plan {
            match step {
                Step::DeadLetter => dead.extend(self.remove(sequence)),
                Step::Deliver => {
                    let handle = handles.take();
                    delivered.extend(self.deliver(sequence, now, visibility_timeout, handle));
                }
            }
        }
        // `dead` is non-empty only when a dead-letter configuration exists.
        let dead = self
            .attributes
            .dead_letter
            .as_ref()
            .filter(|_| !dead.is_empty())
            .map(|dead_letter| DeadBatch {
                queue: dead_letter.queue.clone(),
                messages: dead,
            });
        Received { delivered, dead }
    }

    /// Examines messages in arrival order and decides what happens to each.
    /// The decisions do not depend on each other except through the groups
    /// seen so far and the number delivered, both tracked here.
    fn plan_receive(&self, now: u64, max_messages: u64) -> Vec<(u64, Step)> {
        let max_receive_count = self
            .attributes
            .dead_letter
            .as_ref()
            .map(|dead_letter| dead_letter.max_receive_count);
        // A group stays blocked by any earlier message still in the queue,
        // except one that is moved away.
        let mut blocked: HashSet<&GroupId> = HashSet::new();
        let mut plan = Vec::new();
        let mut delivered = 0;
        for (&sequence, message) in &self.messages {
            if delivered >= max_messages {
                break;
            }
            let group_free = message.group.as_ref().is_none_or(|g| !blocked.contains(g));
            if group_free && message.state(now) == State::Visible {
                if max_receive_count.is_some_and(|max| message.receive_count >= max) {
                    plan.push((sequence, Step::DeadLetter));
                    continue;
                }
                plan.push((sequence, Step::Deliver));
                delivered += 1;
            }
            blocked.extend(message.group.as_ref());
        }
        plan
    }

    fn deliver(
        &mut self,
        sequence: u64,
        now: u64,
        visibility_timeout: u64,
        handle: ReceiptHandle,
    ) -> Option<ReceivedMessage> {
        let message = self.messages.get_mut(&sequence)?;
        if let Some(previous) = message.last_handle.replace(handle) {
            self.handles.remove(&previous);
        }
        self.handles.insert(handle, sequence);
        message.receive_count += 1;
        let first_received_at = *message.first_received_at.get_or_insert(now);
        message.hold = Some(Hold::InFlight {
            until: now.saturating_add(visibility_timeout),
        });
        Some(ReceivedMessage {
            message_id: message.id,
            receipt_handle: handle,
            body: message.body.clone(),
            receive_count: message.receive_count,
            sent_at: message.sent_at,
            first_received_at,
            group_id: message.group.clone(),
            sequence_number: self.attributes.is_fifo().then_some(sequence),
        })
    }

    /// The message `handle` currently refers to, if it is in flight.
    fn in_flight_by_handle(&self, now: u64, handle: ReceiptHandle) -> Result<u64, Error> {
        self.handles
            .get(&handle)
            .copied()
            .filter(|sequence| {
                self.messages
                    .get(sequence)
                    .is_some_and(|message| message.state(now) == State::InFlight)
            })
            .ok_or_else(|| {
                Error::new(
                    ErrorCode::ReceiptHandleExpired,
                    format!("{handle} no longer refers to an in-flight message of this queue"),
                )
            })
    }

    pub fn delete(&mut self, now: u64, handle: ReceiptHandle) -> Result<(), Error> {
        let sequence = self.in_flight_by_handle(now, handle)?;
        self.remove(sequence);
        Ok(())
    }

    pub fn change_visibility(
        &mut self,
        now: u64,
        handle: ReceiptHandle,
        visibility_timeout: u64,
    ) -> Result<(), Error> {
        let sequence = self.in_flight_by_handle(now, handle)?;
        if let Some(message) = self.messages.get_mut(&sequence) {
            message.hold = Some(Hold::InFlight {
                until: now.saturating_add(visibility_timeout),
            });
        }
        Ok(())
    }
}

fn unsupported(message: &str) -> Error {
    Error::new(ErrorCode::UnsupportedOperation, message)
}

fn missing(name: &str) -> Error {
    Error::new(ErrorCode::MissingParameter, format!("{name} is required"))
}
