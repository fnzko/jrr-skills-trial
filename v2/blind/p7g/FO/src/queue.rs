//! One queue: its attributes, its messages and the rules that act on a single queue.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use serde::Serialize;

use crate::error::{Error, ErrorCode};
use crate::fifo::{DedupKey, FifoMeta, FifoState, Original};
use crate::types::{
    Body, DeduplicationId, DelaySeconds, GroupId, MaxMessages, MaxReceiveCount, MessageId,
    QueueName, ReceiptHandle, RetentionSeconds, Time, VisibilityTimeout,
};

const MAX_MESSAGES: usize = 100_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Standard,
    Fifo { content_based_deduplication: bool },
}

impl Kind {
    pub fn is_fifo(self) -> bool {
        matches!(self, Kind::Fifo { .. })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeadLetter {
    pub queue: QueueName,
    pub max_receive_count: MaxReceiveCount,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub visibility_timeout: VisibilityTimeout,
    pub delay: DelaySeconds,
    pub retention: RetentionSeconds,
    pub kind: Kind,
    pub dead_letter: Option<DeadLetter>,
}

/// Position of a message in its queue's arrival order; never reused within one queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ArrivalKey(u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Visible,
    InFlight,
    Delayed,
}

#[derive(Debug, Clone, Copy)]
enum Phase {
    /// Never delayed or delivered, or moved from another queue.
    Idle,
    Delayed {
        until: Time,
    },
    InFlight {
        until: Time,
        handle: ReceiptHandle,
    },
}

impl Phase {
    fn state(self, now: Time) -> State {
        match self {
            Phase::Delayed { until } if now < until => State::Delayed,
            Phase::InFlight { until, .. } if now < until => State::InFlight,
            Phase::Idle | Phase::Delayed { .. } | Phase::InFlight { .. } => State::Visible,
        }
    }
}

#[derive(Debug)]
pub struct Message {
    id: MessageId,
    body: Body,
    sent_at: Time,
    receive_count: u32,
    first_received_at: Option<Time>,
    fifo: Option<FifoMeta>,
    phase: Phase,
}

/// The parts of a send that the queue rules look at.
pub struct NewMessage<'a> {
    pub body: &'a Body,
    pub delay: Option<DelaySeconds>,
    pub group: Option<&'a GroupId>,
    pub deduplication: Option<&'a DeduplicationId>,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Sent {
    pub message_id: MessageId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sequence_number: Option<u64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Delivery {
    message_id: MessageId,
    receipt_handle: ReceiptHandle,
    body: Body,
    receive_count: u32,
    sent_at: Time,
    first_received_at: Time,
    #[serde(flatten)]
    fifo: Option<FifoMeta>,
}

/// Messages that left a queue for its dead-letter queue during one receive.
#[derive(Debug)]
pub struct DeadLettered {
    pub target: QueueName,
    pub messages: Vec<Message>,
}

#[derive(Debug)]
pub struct Received {
    pub delivered: Vec<Delivery>,
    pub dead_lettered: Option<DeadLettered>,
}

#[derive(Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Counts {
    pub visible: usize,
    pub in_flight: usize,
    pub delayed: usize,
}

#[derive(Debug)]
pub struct Queue {
    config: Config,
    messages: BTreeMap<ArrivalKey, Message>,
    /// `(SentAt, key)` of every message, so retention only looks at the oldest ones.
    by_age: BTreeSet<(Time, ArrivalKey)>,
    next_arrival: u64,
    fifo: FifoState,
}

impl Queue {
    pub fn new(config: Config) -> Self {
        Self {
            config,
            messages: BTreeMap::new(),
            by_age: BTreeSet::new(),
            next_arrival: 0,
            fifo: FifoState::default(),
        }
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn set_config(&mut self, config: Config, now: Time) {
        self.config = config;
        self.expire(now);
    }

    /// Drops every message whose retention has run out; call before looking at the messages.
    pub fn expire(&mut self, now: Time) {
        while let Some(&(sent_at, key)) = self.by_age.first() {
            if now < sent_at.plus(self.config.retention) {
                break;
            }
            self.remove(key);
        }
    }

    pub fn counts(&self, now: Time) -> Counts {
        let mut counts = Counts::default();
        for message in self.messages.values() {
            match message.phase.state(now) {
                State::Visible => counts.visible += 1,
                State::InFlight => counts.in_flight += 1,
                State::Delayed => counts.delayed += 1,
            }
        }
        counts
    }

    pub fn purge(&mut self) {
        self.messages.clear();
        self.by_age.clear();
    }

    /// Forgets the dead-letter configuration if it names `deleted`.
    pub fn detach_dead_letter(&mut self, deleted: &str) {
        if self
            .config
            .dead_letter
            .as_ref()
            .is_some_and(|dead_letter| dead_letter.queue.as_str() == deleted)
        {
            self.config.dead_letter = None;
        }
    }

    pub fn send(
        &mut self,
        now: Time,
        new: &NewMessage<'_>,
        allocate_id: impl FnOnce() -> MessageId,
    ) -> Result<Sent, Error> {
        let fifo_identity = match self.config.kind {
            Kind::Standard => {
                if new.group.is_some() || new.deduplication.is_some() {
                    return Err(Error::unsupported(
                        "GroupId and DeduplicationId are only for FIFO queues",
                    ));
                }
                None
            }
            Kind::Fifo {
                content_based_deduplication,
            } => {
                if new.delay.is_some() {
                    return Err(Error::unsupported(
                        "DelaySeconds cannot be set per message on a FIFO queue",
                    ));
                }
                let group = new.group.ok_or_else(|| Error::missing("GroupId"))?;
                let key = match (new.deduplication, content_based_deduplication) {
                    (Some(id), _) => DedupKey::from_id(id),
                    (None, true) => DedupKey::from_body(new.body),
                    (None, false) => return Err(Error::missing("DeduplicationId")),
                };
                if let Some(original) = self.fifo.find_duplicate(&key, now) {
                    return Ok(Sent {
                        message_id: original.message_id,
                        sequence_number: Some(original.sequence_number),
                    });
                }
                Some((group, key))
            }
        };
        if self.messages.len() >= MAX_MESSAGES {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                format!("the queue already holds {MAX_MESSAGES} messages"),
            ));
        }

        let id = allocate_id();
        let delay = new.delay.unwrap_or(self.config.delay);
        let phase = if delay.get() == 0 {
            Phase::Idle
        } else {
            Phase::Delayed {
                until: now.plus(delay),
            }
        };
        let fifo = fifo_identity.map(|(group, key)| {
            let sequence_number = self.fifo.next_sequence();
            let original = Original {
                message_id: id,
                sequence_number,
            };
            self.fifo.record(key, now, original);
            FifoMeta {
                group_id: group.clone(),
                sequence_number,
            }
        });
        let sequence_number = fifo.as_ref().map(|meta| meta.sequence_number);
        self.push(Message {
            id,
            body: new.body.clone(),
            sent_at: now,
            receive_count: 0,
            first_received_at: None,
            fifo,
            phase,
        });
        Ok(Sent {
            message_id: id,
            sequence_number,
        })
    }

    /// Adds messages moved from another queue. The limit and deduplication do not apply, and
    /// retention keeps counting from the original `SentAt`.
    pub fn accept_dead_lettered(&mut self, messages: Vec<Message>, now: Time) {
        for moved in messages {
            let fifo = moved.fifo.map(|meta| FifoMeta {
                group_id: meta.group_id,
                sequence_number: self.fifo.next_sequence(),
            });
            self.push(Message {
                receive_count: 0,
                first_received_at: None,
                fifo,
                phase: Phase::Idle,
                ..moved
            });
        }
        self.expire(now);
    }

    /// Delivers up to `max` eligible messages in arrival order. `issue` hands out the receipt
    /// handle for each delivery.
    pub fn receive(
        &mut self,
        now: Time,
        max: MaxMessages,
        visibility: VisibilityTimeout,
        mut issue: impl FnMut(ArrivalKey) -> ReceiptHandle,
    ) -> Received {
        let max_receive_count = self
            .config
            .dead_letter
            .as_ref()
            .map(|dead_letter| dead_letter.max_receive_count.get());
        // Groups that have an earlier message still in the queue.
        let mut blocked: HashSet<GroupId> = HashSet::new();
        let mut delivered = Vec::new();
        let mut to_dead_letter = Vec::new();
        let mut remaining = max.get();

        for (&key, message) in &mut self.messages {
            if remaining == 0 {
                break;
            }
            if let Some(meta) = &message.fifo {
                if blocked.contains(&meta.group_id) {
                    continue;
                }
                // Whatever happens next, a message that stays keeps its group blocked; one
                // that is moved away does not, so it is unblocked again below.
                blocked.insert(meta.group_id.clone());
            }
            if message.phase.state(now) != State::Visible {
                continue;
            }
            if max_receive_count.is_some_and(|limit| message.receive_count >= limit) {
                if let Some(meta) = &message.fifo {
                    blocked.remove(&meta.group_id);
                }
                to_dead_letter.push(key);
                continue;
            }

            message.receive_count += 1;
            let first_received_at = *message.first_received_at.get_or_insert(now);
            let handle = issue(key);
            message.phase = Phase::InFlight {
                until: now.plus(visibility),
                handle,
            };
            delivered.push(Delivery {
                message_id: message.id,
                receipt_handle: handle,
                body: message.body.clone(),
                receive_count: message.receive_count,
                sent_at: message.sent_at,
                first_received_at,
                fifo: message.fifo.clone(),
            });
            remaining -= 1;
        }

        let target = self
            .config
            .dead_letter
            .as_ref()
            .filter(|_| !to_dead_letter.is_empty())
            .map(|dead_letter| dead_letter.queue.clone());
        let dead_lettered = target.map(|target| DeadLettered {
            target,
            messages: to_dead_letter
                .into_iter()
                .map(|key| {
                    self.remove(key)
                        .expect("the key was read from this queue during this call")
                })
                .collect(),
        });
        Received {
            delivered,
            dead_lettered,
        }
    }

    /// Deletes the message if `handle` is its latest handle and it is in flight.
    pub fn acknowledge(
        &mut self,
        key: ArrivalKey,
        handle: ReceiptHandle,
        now: Time,
    ) -> Result<(), Error> {
        self.in_flight_with(key, handle, now)?;
        self.remove(key);
        Ok(())
    }

    /// Moves the end of the visibility timeout of the message to `now + timeout`.
    pub fn change_visibility(
        &mut self,
        key: ArrivalKey,
        handle: ReceiptHandle,
        timeout: VisibilityTimeout,
        now: Time,
    ) -> Result<(), Error> {
        self.in_flight_with(key, handle, now)?.phase = Phase::InFlight {
            until: now.plus(timeout),
            handle,
        };
        Ok(())
    }

    fn in_flight_with(
        &mut self,
        key: ArrivalKey,
        handle: ReceiptHandle,
        now: Time,
    ) -> Result<&mut Message, Error> {
        self.messages
            .get_mut(&key)
            .filter(|message| match message.phase {
                Phase::InFlight {
                    until,
                    handle: latest,
                } => latest == handle && now < until,
                Phase::Idle | Phase::Delayed { .. } => false,
            })
            .ok_or_else(|| {
                Error::new(
                    ErrorCode::ReceiptHandleExpired,
                    format!("receipt handle {handle} no longer refers to an in-flight message"),
                )
            })
    }

    fn push(&mut self, message: Message) {
        let key = ArrivalKey(self.next_arrival);
        self.next_arrival += 1;
        self.by_age.insert((message.sent_at, key));
        self.messages.insert(key, message);
    }

    fn remove(&mut self, key: ArrivalKey) -> Option<Message> {
        let message = self.messages.remove(&key)?;
        self.by_age.remove(&(message.sent_at, key));
        Some(message)
    }
}
