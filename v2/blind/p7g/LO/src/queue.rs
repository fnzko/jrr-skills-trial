//! A queue and its messages. Everything here is local to one queue; commands that involve
//! several queues (dead-letter moves, deletion) are sequenced by the broker.

use std::collections::{HashMap, HashSet};

use serde::Serialize;

use crate::error::{Error, ErrorCode};
use crate::time::{Instant, Seconds};
use crate::values::{
    DeduplicationId, DelaySeconds, GroupId, MaxMessages, MaxReceiveCount, MessageId, QueueName,
    ReceiptHandle, RetentionSeconds, SequenceNumber, VisibilityTimeout,
};

pub const MAX_MESSAGES_PER_QUEUE: usize = 100_000;

/// A send repeating a deduplication id within this long of the accepted one is a duplicate.
const DEDUPLICATION_WINDOW: Seconds = Seconds::new(300);

pub const DEFAULT_VISIBILITY_TIMEOUT: VisibilityTimeout = VisibilityTimeout::constant(30);
pub const DEFAULT_DELAY: DelaySeconds = DelaySeconds::constant(0);
pub const DEFAULT_RETENTION: RetentionSeconds = RetentionSeconds::constant(345_600);

/// Where messages go once they were received too often.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DeadLetter {
    #[serde(rename = "DeadLetterQueue")]
    pub queue: QueueName,
    #[serde(rename = "MaxReceiveCount")]
    pub max_receive_count: MaxReceiveCount,
}

/// A dead-letter configuration as named by a request, before the queue is looked up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeadLetterRequest<'a> {
    pub queue: &'a str,
    pub max_receive_count: MaxReceiveCount,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueType {
    Standard,
    Fifo { content_based_deduplication: bool },
}

/// Every attribute a new queue gets, defaults already applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Requested<'a> {
    pub visibility_timeout: VisibilityTimeout,
    pub delay: DelaySeconds,
    pub retention: RetentionSeconds,
    pub queue_type: QueueType,
    pub dead_letter: Option<DeadLetterRequest<'a>>,
}

/// Attributes to change on an existing queue; `None` keeps the current value. The dead-letter
/// configuration can be replaced but not removed.
#[derive(Debug, Default)]
pub struct Update {
    pub visibility_timeout: Option<VisibilityTimeout>,
    pub delay: Option<DelaySeconds>,
    pub retention: Option<RetentionSeconds>,
    pub content_based_deduplication: Option<bool>,
    pub dead_letter: Option<DeadLetter>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FifoPosition {
    #[serde(rename = "GroupId")]
    pub group: GroupId,
    #[serde(rename = "SequenceNumber")]
    pub sequence: SequenceNumber,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Hold {
    None,
    Delayed {
        until: Instant,
    },
    InFlight {
        handle: ReceiptHandle,
        until: Instant,
    },
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
    sent_at: Instant,
    receive_count: u64,
    first_received_at: Option<Instant>,
    fifo: Option<FifoPosition>,
    hold: Hold,
}

impl Message {
    fn state(&self, now: Instant) -> State {
        match self.hold {
            Hold::Delayed { until } if now < until => State::Delayed,
            Hold::InFlight { until, .. } if now < until => State::InFlight,
            Hold::None | Hold::Delayed { .. } | Hold::InFlight { .. } => State::Visible,
        }
    }

    pub fn id(&self) -> MessageId {
        self.id
    }

    pub fn body(&self) -> &str {
        &self.body
    }

    pub fn sent_at(&self) -> Instant {
        self.sent_at
    }

    pub fn receive_count(&self) -> u64 {
        self.receive_count
    }

    pub fn first_received_at(&self) -> Option<Instant> {
        self.first_received_at
    }

    pub fn fifo(&self) -> Option<&FifoPosition> {
        self.fifo.as_ref()
    }

    /// The handle of the latest delivery, while the message is in flight.
    pub fn receipt_handle(&self) -> Option<ReceiptHandle> {
        match self.hold {
            Hold::InFlight { handle, .. } => Some(handle),
            Hold::None | Hold::Delayed { .. } => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Counts {
    pub visible: usize,
    pub in_flight: usize,
    pub delayed: usize,
}

#[derive(Debug)]
struct Accepted {
    at: Instant,
    message: MessageId,
    sequence: SequenceNumber,
}

#[derive(Debug)]
struct FifoState {
    content_based_deduplication: bool,
    next_sequence: SequenceNumber,
    accepted: HashMap<DeduplicationId, Accepted>,
}

#[derive(Debug)]
enum Kind {
    Standard,
    Fifo(FifoState),
}

#[derive(Debug)]
pub struct Queue {
    visibility_timeout: VisibilityTimeout,
    delay: DelaySeconds,
    retention: RetentionSeconds,
    dead_letter: Option<DeadLetter>,
    kind: Kind,
    messages: Vec<Message>,
}

/// What `send` did with a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sent {
    pub message_id: MessageId,
    pub sequence: Option<SequenceNumber>,
    /// False when the send repeated a deduplication id and nothing was added.
    pub accepted: bool,
}

/// The result of one `receive` scan. Indices point into the queue's messages and stay valid
/// until the queue is next changed.
#[derive(Debug)]
pub struct Received {
    pub delivered: Vec<usize>,
    pub dead: Vec<Message>,
}

impl Queue {
    pub fn new(requested: &Requested<'_>, dead_letter: Option<DeadLetter>) -> Self {
        let kind = match requested.queue_type {
            QueueType::Standard => Kind::Standard,
            QueueType::Fifo {
                content_based_deduplication,
            } => Kind::Fifo(FifoState {
                content_based_deduplication,
                next_sequence: SequenceNumber::FIRST,
                accepted: HashMap::new(),
            }),
        };
        Self {
            visibility_timeout: requested.visibility_timeout,
            delay: requested.delay,
            retention: requested.retention,
            dead_letter,
            kind,
            messages: Vec::new(),
        }
    }

    pub fn is_fifo(&self) -> bool {
        matches!(self.kind, Kind::Fifo(_))
    }

    pub fn queue_type(&self) -> QueueType {
        match &self.kind {
            Kind::Standard => QueueType::Standard,
            Kind::Fifo(state) => QueueType::Fifo {
                content_based_deduplication: state.content_based_deduplication,
            },
        }
    }

    pub fn visibility_timeout(&self) -> VisibilityTimeout {
        self.visibility_timeout
    }

    pub fn delay(&self) -> DelaySeconds {
        self.delay
    }

    pub fn retention(&self) -> RetentionSeconds {
        self.retention
    }

    pub fn dead_letter(&self) -> Option<&DeadLetter> {
        self.dead_letter.as_ref()
    }

    pub fn messages(&self) -> &[Message] {
        &self.messages
    }

    /// True when every attribute equals the one requested.
    pub fn matches(&self, requested: &Requested<'_>) -> bool {
        let same_dead_letter = match (&self.dead_letter, &requested.dead_letter) {
            (None, None) => true,
            (Some(have), Some(want)) => {
                have.queue.as_str() == want.queue
                    && have.max_receive_count == want.max_receive_count
            }
            (None, Some(_)) | (Some(_), None) => false,
        };
        same_dead_letter
            && self.visibility_timeout == requested.visibility_timeout
            && self.delay == requested.delay
            && self.retention == requested.retention
            && self.queue_type() == requested.queue_type
    }

    pub fn counts(&self, now: Instant) -> Counts {
        let mut counts = Counts {
            visible: 0,
            in_flight: 0,
            delayed: 0,
        };
        for message in &self.messages {
            match message.state(now) {
                State::Visible => counts.visible += 1,
                State::InFlight => counts.in_flight += 1,
                State::Delayed => counts.delayed += 1,
            }
        }
        counts
    }

    /// Applies an already validated update. A shorter retention takes effect at once.
    pub fn update(&mut self, update: Update, now: Instant) {
        if let Some(timeout) = update.visibility_timeout {
            self.visibility_timeout = timeout;
        }
        if let Some(delay) = update.delay {
            self.delay = delay;
        }
        if let Some(retention) = update.retention {
            self.retention = retention;
            self.expire(now);
        }
        if let (Some(enabled), Kind::Fifo(state)) =
            (update.content_based_deduplication, &mut self.kind)
        {
            state.content_based_deduplication = enabled;
        }
        if update.dead_letter.is_some() {
            self.dead_letter = update.dead_letter;
        }
    }

    pub fn clear_dead_letter(&mut self) {
        self.dead_letter = None;
    }

    pub fn purge(&mut self) {
        self.messages.clear();
    }

    /// Drops what the clock reaching `now` made obsolete. Retention only changes with the
    /// clock or with the queue's attributes, so nothing else needs to look for expired
    /// messages.
    pub fn advance_to(&mut self, now: Instant) {
        self.expire(now);
        if let Kind::Fifo(state) = &mut self.kind {
            state
                .accepted
                .retain(|_, accepted| now.since(accepted.at) < DEDUPLICATION_WINDOW);
        }
    }

    fn expire(&mut self, now: Instant) {
        let retention = self.retention.seconds();
        self.messages
            .retain(|message| now < message.sent_at.after(retention));
    }

    /// Applies the send rules of the spec in order. `next_id` is used only when the message
    /// is accepted.
    pub fn send(
        &mut self,
        next_id: MessageId,
        now: Instant,
        body: &str,
        delay: Option<DelaySeconds>,
        group: Option<GroupId>,
        deduplication: Option<DeduplicationId>,
    ) -> Result<Sent, Error> {
        let Self {
            delay: default_delay,
            kind,
            messages,
            ..
        } = self;
        let (fifo, sequence) = match kind {
            Kind::Standard => {
                if group.is_some() || deduplication.is_some() {
                    return Err(Error::new(
                        ErrorCode::UnsupportedOperation,
                        "GroupId and DeduplicationId are only supported by FIFO queues",
                    ));
                }
                check_room(messages)?;
                (None, None)
            }
            Kind::Fifo(state) => {
                if delay.is_some() {
                    return Err(Error::new(
                        ErrorCode::UnsupportedOperation,
                        "DelaySeconds is not supported per message on FIFO queues",
                    ));
                }
                let group = group.ok_or_else(|| Error::missing("GroupId"))?;
                if deduplication.is_none() && !state.content_based_deduplication {
                    return Err(Error::missing("DeduplicationId"));
                }
                let key = deduplication.as_ref().map_or(body, DeduplicationId::as_str);
                if let Some(original) = state
                    .accepted
                    .get(key)
                    .filter(|original| now.since(original.at) < DEDUPLICATION_WINDOW)
                {
                    return Ok(Sent {
                        message_id: original.message,
                        sequence: Some(original.sequence),
                        accepted: false,
                    });
                }
                check_room(messages)?;
                let sequence = state.next_sequence.take_next();
                state.accepted.insert(
                    deduplication.unwrap_or_else(|| DeduplicationId::from_body(body)),
                    Accepted {
                        at: now,
                        message: next_id,
                        sequence,
                    },
                );
                (Some(FifoPosition { group, sequence }), Some(sequence))
            }
        };
        let delay = delay.unwrap_or(*default_delay).seconds();
        messages.push(Message {
            id: next_id,
            body: body.to_owned(),
            sent_at: now,
            receive_count: 0,
            first_received_at: None,
            fifo,
            hold: Hold::Delayed {
                until: now.after(delay),
            },
        });
        Ok(Sent {
            message_id: next_id,
            sequence,
            accepted: true,
        })
    }

    /// Scans the messages in arrival order, delivering up to `max` of them. Messages over the
    /// receive limit of the dead-letter configuration are removed and returned in `dead`.
    pub fn receive(
        &mut self,
        now: Instant,
        max: MaxMessages,
        timeout: VisibilityTimeout,
        next_handle: &mut ReceiptHandle,
    ) -> Received {
        let max = usize::try_from(max.get()).expect("MaxMessages is at most 10");
        let mut delivered = Vec::new();
        let mut dead = Vec::new();
        // Groups with a message that stays in the queue ahead of the one being examined.
        let mut occupied: HashSet<GroupId> = HashSet::new();
        let mut index = 0;
        while delivered.len() < max {
            let Some(message) = self.messages.get_mut(index) else {
                break;
            };
            let group_free = message
                .fifo
                .as_ref()
                .is_none_or(|position| !occupied.contains(&position.group));
            if group_free && message.state(now) == State::Visible {
                let over_limit = self.dead_letter.as_ref().is_some_and(|config| {
                    message.receive_count >= u64::from(config.max_receive_count.get())
                });
                if over_limit {
                    dead.push(self.messages.remove(index));
                    continue;
                }
                message.receive_count += 1;
                message.first_received_at.get_or_insert(now);
                message.hold = Hold::InFlight {
                    handle: next_handle.take_next(),
                    until: now.after(timeout.seconds()),
                };
                delivered.push(index);
            }
            if let Some(position) = &message.fifo
                && !occupied.contains(&position.group)
            {
                occupied.insert(position.group.clone());
            }
            index += 1;
        }
        Received { delivered, dead }
    }

    /// Adds a message that another queue gave up on, as a fresh and immediately visible one.
    pub fn add_dead(&mut self, mut message: Message, now: Instant) {
        message.receive_count = 0;
        message.first_received_at = None;
        message.hold = Hold::None;
        if let (Some(position), Kind::Fifo(state)) = (&mut message.fifo, &mut self.kind) {
            position.sequence = state.next_sequence.take_next();
        }
        if now < message.sent_at.after(self.retention.seconds()) {
            self.messages.push(message);
        }
    }

    /// The position of the message `handle` was last issued for, if it is in flight now.
    fn in_flight(&self, handle: ReceiptHandle, now: Instant) -> Result<usize, Error> {
        self.messages
            .iter()
            .position(|m| m.state(now) == State::InFlight && m.receipt_handle() == Some(handle))
            .ok_or_else(|| {
                Error::new(
                    ErrorCode::ReceiptHandleExpired,
                    "the receipt handle no longer refers to a message in flight in this queue",
                )
            })
    }

    pub fn delete(&mut self, handle: ReceiptHandle, now: Instant) -> Result<(), Error> {
        let index = self.in_flight(handle, now)?;
        self.messages.remove(index);
        Ok(())
    }

    pub fn change_visibility(
        &mut self,
        handle: ReceiptHandle,
        now: Instant,
        timeout: VisibilityTimeout,
    ) -> Result<(), Error> {
        let index = self.in_flight(handle, now)?;
        self.messages[index].hold = Hold::InFlight {
            handle,
            until: now.after(timeout.seconds()),
        };
        Ok(())
    }
}

fn check_room(messages: &[Message]) -> Result<(), Error> {
    if messages.len() >= MAX_MESSAGES_PER_QUEUE {
        return Err(Error::new(
            ErrorCode::LimitExceeded,
            format!("the queue already holds {MAX_MESSAGES_PER_QUEUE} messages"),
        ));
    }
    Ok(())
}
