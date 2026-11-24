//! One queue: its attributes, its messages and the rules that move them
//! between states.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use serde::Serialize;

use crate::command::{AttributeSet, NewMessage};
use crate::error::{Error, Limit, Problem};
use crate::types::{
    Body, Counter, DedupKey, DelaySeconds, GroupId, MaxMessages, MaxReceiveCount, MessageId,
    QueueName, ReceiptHandle, RetentionSeconds, SequenceNumber, Time, VisibilityTimeout,
};

const MAX_MESSAGES_PER_QUEUE: usize = 100_000;
const DEDUPLICATION_WINDOW_SECONDS: u64 = 300;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Standard,
    Fifo { content_based_deduplication: bool },
}

impl Kind {
    pub fn from_flags(fifo: bool, content_based_deduplication: bool) -> Result<Kind, Error> {
        match (fifo, content_based_deduplication) {
            (true, content_based_deduplication) => Ok(Kind::Fifo {
                content_based_deduplication,
            }),
            (false, false) => Ok(Kind::Standard),
            (false, true) => Err(content_based_requires_fifo()),
        }
    }

    pub fn is_fifo(self) -> bool {
        matches!(self, Kind::Fifo { .. })
    }
}

pub fn content_based_requires_fifo() -> Error {
    Error::invalid(
        "ContentBasedDeduplication",
        Problem::Conflict("content-based deduplication needs a FIFO queue"),
    )
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DeadLetter {
    #[serde(rename = "DeadLetterQueue")]
    pub queue: QueueName,
    #[serde(rename = "MaxReceiveCount")]
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

impl Config {
    /// Applies `attributes` over the defaults.
    pub fn from_attributes(
        attributes: &AttributeSet,
        kind: Kind,
        dead_letter: Option<DeadLetter>,
    ) -> Config {
        Config {
            visibility_timeout: attributes.visibility_timeout.unwrap_or_default(),
            delay: attributes.delay.unwrap_or_default(),
            retention: attributes.retention.unwrap_or_default(),
            kind,
            dead_letter,
        }
    }

    /// Whether a `CreateQueue` request for `kind` with these attributes
    /// describes exactly this configuration.
    pub fn is_requested_by(&self, attributes: &AttributeSet, kind: Kind) -> bool {
        let requested_dead_letter = attributes
            .dead_letter_queue
            .as_deref()
            .zip(attributes.max_receive_count);
        let current_dead_letter = self
            .dead_letter
            .as_ref()
            .map(|d| (d.queue.as_str(), d.max_receive_count));
        self.visibility_timeout == attributes.visibility_timeout.unwrap_or_default()
            && self.delay == attributes.delay.unwrap_or_default()
            && self.retention == attributes.retention.unwrap_or_default()
            && self.kind == kind
            && current_dead_letter == requested_dead_letter
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Delayed,
    InFlight,
    Visible,
}

/// Why a message is not visible before `available_at`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Hold {
    Delayed,
    InFlight(ReceiptHandle),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FifoPosition {
    #[serde(rename = "GroupId")]
    pub group: GroupId,
    #[serde(rename = "SequenceNumber")]
    pub sequence: SequenceNumber,
}

#[derive(Debug)]
pub struct Message {
    id: MessageId,
    body: Body,
    sent_at: Time,
    receive_count: u32,
    first_received_at: Option<Time>,
    fifo: Option<FifoPosition>,
    available_at: Time,
    hold: Hold,
}

impl Message {
    fn phase(&self, now: Time) -> Phase {
        match self.hold {
            _ if now >= self.available_at => Phase::Visible,
            Hold::Delayed => Phase::Delayed,
            Hold::InFlight(_) => Phase::InFlight,
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Counts {
    pub visible: usize,
    pub in_flight: usize,
    pub delayed: usize,
}

/// A message handed to a consumer, as `ReceiveMessage` reports it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Delivered {
    pub message_id: MessageId,
    pub receipt_handle: ReceiptHandle,
    pub body: Body,
    pub receive_count: u32,
    pub sent_at: Time,
    pub first_received_at: Time,
    #[serde(flatten)]
    pub fifo: Option<FifoPosition>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Accepted {
    pub id: MessageId,
    pub sequence: Option<SequenceNumber>,
}

/// Messages that left a queue for its dead-letter queue.
#[derive(Debug)]
pub struct DeadLettered {
    pub target: QueueName,
    pub messages: Vec<Message>,
}

#[derive(Debug)]
pub struct Received {
    pub delivered: Vec<Delivered>,
    pub dead_lettered: Option<DeadLettered>,
}

/// An accepted FIFO send, remembered for deduplication.
#[derive(Debug, Clone, Copy)]
struct SeenSend {
    at: Time,
    accepted: Accepted,
}

#[derive(Debug, Default)]
struct FifoState {
    next_sequence: u64,
    seen: HashMap<DedupKey, SeenSend>,
}

/// Position of a message in its queue's arrival order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Arrival(u64);

#[derive(Debug)]
pub struct Queue {
    config: Config,
    messages: BTreeMap<Arrival, Message>,
    /// The same messages ordered by `sent_at`, so retention only looks at the oldest.
    by_sent_at: BTreeSet<(Time, Arrival)>,
    /// Latest receipt handle of each message that has ever been delivered and still exists.
    handles: HashMap<ReceiptHandle, Arrival>,
    next_arrival: u64,
    fifo: FifoState,
}

impl Queue {
    pub fn new(config: Config) -> Queue {
        Queue {
            config,
            messages: BTreeMap::new(),
            by_sent_at: BTreeSet::new(),
            handles: HashMap::new(),
            next_arrival: 0,
            fifo: FifoState::default(),
        }
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn config_mut(&mut self) -> &mut Config {
        &mut self.config
    }

    /// Removes the messages whose retention has run out. Every operation
    /// calls this first, so no caller can observe an expired message.
    pub fn expire(&mut self, now: Time) {
        let retention = self.config.retention.get();
        while let Some(&(sent_at, arrival)) = self.by_sent_at.first() {
            if sent_at.plus(retention) > now {
                break;
            }
            self.take(arrival);
        }
    }

    pub fn counts(&self, now: Time) -> Counts {
        self.messages
            .values()
            .fold(Counts::default(), |mut counts, message| {
                match message.phase(now) {
                    Phase::Visible => counts.visible += 1,
                    Phase::InFlight => counts.in_flight += 1,
                    Phase::Delayed => counts.delayed += 1,
                }
                counts
            })
    }

    pub fn purge(&mut self) {
        self.messages.clear();
        self.by_sent_at.clear();
        self.handles.clear();
    }

    fn take(&mut self, arrival: Arrival) -> Message {
        let message = self
            .messages
            .remove(&arrival)
            .expect("arrivals come from the message map or its mirror index");
        self.by_sent_at.remove(&(message.sent_at, arrival));
        if let Hold::InFlight(handle) = message.hold {
            self.handles.remove(&handle);
        }
        message
    }

    fn insert(&mut self, message: Message) {
        let arrival = Arrival(self.next_arrival);
        self.next_arrival += 1;
        self.by_sent_at.insert((message.sent_at, arrival));
        self.messages.insert(arrival, message);
    }

    fn next_position(&mut self, group: GroupId) -> FifoPosition {
        self.fifo.next_sequence += 1;
        FifoPosition {
            group,
            sequence: SequenceNumber::from_counter(self.fifo.next_sequence),
        }
    }

    pub fn send(
        &mut self,
        now: Time,
        new: NewMessage,
        ids: &mut Counter,
    ) -> Result<Accepted, Error> {
        let NewMessage {
            body,
            delay,
            group,
            deduplication,
        } = new;
        let queue_delay = self.config.delay;
        let (fifo_group, dedup_key, delay) = match self.config.kind {
            Kind::Standard => {
                if group.is_some() || deduplication.is_some() {
                    return Err(Error::UnsupportedOperation(
                        "GroupId and DeduplicationId need a FIFO queue",
                    ));
                }
                (None, None, delay.unwrap_or(queue_delay))
            }
            Kind::Fifo {
                content_based_deduplication,
            } => {
                if delay.is_some() {
                    return Err(Error::UnsupportedOperation(
                        "a FIFO queue takes no per-message delay",
                    ));
                }
                let group = group.ok_or(Error::MissingParameter("GroupId"))?;
                if deduplication.is_none() && !content_based_deduplication {
                    return Err(Error::MissingParameter("DeduplicationId"));
                }
                let key = DedupKey::of(deduplication.as_ref(), &body);
                if let Some(seen) = self.fifo.seen.get(&key)
                    && now.since(seen.at) < DEDUPLICATION_WINDOW_SECONDS
                {
                    return Ok(seen.accepted);
                }
                (Some(group), Some(key), queue_delay)
            }
        };
        if self.messages.len() >= MAX_MESSAGES_PER_QUEUE {
            return Err(Error::LimitExceeded(Limit::Messages));
        }
        let id = MessageId::from_counter(ids.advance());
        let fifo = fifo_group.map(|group| self.next_position(group));
        let accepted = Accepted {
            id,
            sequence: fifo.as_ref().map(|f| f.sequence),
        };
        if let Some(key) = dedup_key {
            self.fifo.seen.insert(key, SeenSend { at: now, accepted });
        }
        self.insert(Message {
            id,
            body,
            sent_at: now,
            receive_count: 0,
            first_received_at: None,
            fifo,
            available_at: now.plus(delay.get()),
            hold: Hold::Delayed,
        });
        Ok(accepted)
    }

    /// Adds a message that was moved here from another queue. It starts over
    /// as a fresh, visible message but keeps its age.
    pub fn accept_dead_letter(&mut self, mut message: Message, now: Time) {
        message.receive_count = 0;
        message.first_received_at = None;
        message.available_at = message.sent_at;
        message.hold = Hold::Delayed;
        message.fifo = message.fifo.take().map(|old| self.next_position(old.group));
        self.insert(message);
        self.expire(now);
    }

    pub fn receive(
        &mut self,
        now: Time,
        max: MaxMessages,
        visibility_timeout: VisibilityTimeout,
        receipts: &mut Counter,
    ) -> Received {
        let max = usize::try_from(max.get()).expect("MaxMessages is at most 10");
        let mut delivered = Vec::new();
        let mut to_move = Vec::new();
        let mut groups_in_queue: HashSet<GroupId> = HashSet::new();
        for (&arrival, message) in &mut self.messages {
            if delivered.len() == max {
                break;
            }
            let group_blocked = message
                .fifo
                .as_ref()
                .is_some_and(|f| groups_in_queue.contains(f.group.as_str()));
            let eligible = !group_blocked && message.phase(now) == Phase::Visible;
            if eligible
                && let Some(dead_letter) = &self.config.dead_letter
                && message.receive_count >= dead_letter.max_receive_count.get()
            {
                to_move.push(arrival);
                continue;
            }
            if let Some(f) = &message.fifo
                && !group_blocked
            {
                groups_in_queue.insert(f.group.clone());
            }
            if !eligible {
                continue;
            }
            message.receive_count += 1;
            let first_received_at = *message.first_received_at.get_or_insert(now);
            let handle = ReceiptHandle::from_counter(receipts.advance());
            if let Hold::InFlight(previous) = message.hold {
                self.handles.remove(&previous);
            }
            message.hold = Hold::InFlight(handle);
            message.available_at = now.plus(visibility_timeout.get());
            self.handles.insert(handle, arrival);
            delivered.push(Delivered {
                message_id: message.id,
                receipt_handle: handle,
                body: message.body.clone(),
                receive_count: message.receive_count,
                sent_at: message.sent_at,
                first_received_at,
                fifo: message.fifo.clone(),
            });
        }
        let target = self
            .config
            .dead_letter
            .as_ref()
            .map(|dead_letter| dead_letter.queue.clone())
            .filter(|_| !to_move.is_empty());
        let dead_lettered = target.map(|target| DeadLettered {
            target,
            messages: to_move
                .into_iter()
                .map(|arrival| self.take(arrival))
                .collect(),
        });
        Received {
            delivered,
            dead_lettered,
        }
    }

    fn in_flight(&self, handle: ReceiptHandle, now: Time) -> Result<Arrival, Error> {
        self.handles
            .get(&handle)
            .copied()
            .filter(|arrival| {
                self.messages
                    .get(arrival)
                    .is_some_and(|m| m.phase(now) == Phase::InFlight)
            })
            .ok_or(Error::ReceiptHandleExpired)
    }

    pub fn delete(&mut self, handle: ReceiptHandle, now: Time) -> Result<(), Error> {
        let arrival = self.in_flight(handle, now)?;
        self.take(arrival);
        Ok(())
    }

    pub fn change_visibility(
        &mut self,
        handle: ReceiptHandle,
        now: Time,
        visibility_timeout: VisibilityTimeout,
    ) -> Result<(), Error> {
        let arrival = self.in_flight(handle, now)?;
        let message = self
            .messages
            .get_mut(&arrival)
            .expect("in_flight returns the arrival of a live message");
        message.available_at = now.plus(visibility_timeout.get());
        Ok(())
    }
}
