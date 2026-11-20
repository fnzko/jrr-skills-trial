use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};

use serde::Serialize;

use crate::error::Error;
use crate::request::NewMessage;
use crate::types::{
    Counter, DeduplicationKey, GroupId, MessageId, QueueName, ReceiptHandle, Seconds,
    SequenceNumber, Timestamp,
};

/// The attributes of SPEC.md section 4, as one queue holds them.
#[derive(Debug, PartialEq, Eq)]
pub struct QueueAttributes {
    pub visibility_timeout: Seconds,
    pub delay: Seconds,
    pub retention: Seconds,
    pub kind: QueueKind,
    pub dead_letter: Option<DeadLetter>,
}

impl QueueAttributes {
    pub const DEFAULT_VISIBILITY_TIMEOUT: Seconds = Seconds::new(30);
    pub const DEFAULT_DELAY: Seconds = Seconds::new(0);
    pub const DEFAULT_RETENTION: Seconds = Seconds::new(345_600);
}

/// `Fifo` and `ContentBasedDeduplication` together: only a FIFO queue can
/// deduplicate by content.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueKind {
    Standard,
    Fifo { content_based_deduplication: bool },
}

impl QueueKind {
    pub fn is_fifo(self) -> bool {
        matches!(self, QueueKind::Fifo { .. })
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct DeadLetter {
    /// Always names an existing queue: deleting a queue clears every
    /// configuration that points at it.
    pub queue: QueueName,
    pub max_receive_count: u32,
}

/// The most messages a queue may hold, counting every state.
pub const MAX_MESSAGES: usize = 100_000;

/// How long an accepted deduplication key turns away repeats.
const DEDUPLICATION_WINDOW: Seconds = Seconds::new(300);

/// A message's place in its queue's arrival order; never reused within a queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Arrival(u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageState {
    Delayed,
    InFlight,
    Visible,
}

/// The number of messages in each state, as `GetQueueAttributes` reports it.
#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Counts {
    visible: u64,
    in_flight: u64,
    delayed: u64,
}

/// What hides a message until a moment, if that moment is still ahead.
#[derive(Debug, Clone, Copy)]
enum Hold {
    /// Not delivered since it arrived; hidden by its send delay.
    Arrived { until: Timestamp },
    /// Delivered; in flight under `handle` until `until`.
    Delivered {
        until: Timestamp,
        handle: ReceiptHandle,
    },
}

#[derive(Debug)]
struct FifoTag {
    group: GroupId,
    sequence: SequenceNumber,
}

#[derive(Debug)]
pub struct Message {
    id: MessageId,
    body: String,
    sent_at: Timestamp,
    receive_count: u32,
    first_received_at: Option<Timestamp>,
    /// Present exactly in FIFO queues.
    fifo: Option<FifoTag>,
    hold: Hold,
}

impl Message {
    fn state(&self, now: Timestamp) -> MessageState {
        match self.hold {
            Hold::Arrived { until } if now < until => MessageState::Delayed,
            Hold::Delivered { until, .. } if now < until => MessageState::InFlight,
            _ => MessageState::Visible,
        }
    }

    fn handle(&self) -> Option<ReceiptHandle> {
        match self.hold {
            Hold::Arrived { .. } => None,
            Hold::Delivered { handle, .. } => Some(handle),
        }
    }
}

/// The reply to a successful send: the new message, or the original one when
/// the send was a duplicate.
#[derive(Debug, Clone, Copy)]
pub struct Sent {
    pub id: MessageId,
    pub sequence: Option<SequenceNumber>,
}

/// The deduplication keys a FIFO queue accepted within the last window.
#[derive(Debug, Default)]
struct Deduplication {
    recent: HashMap<DeduplicationKey, Sent>,
    /// The keys of `recent` in acceptance order, which is also time order, so
    /// stale keys are dropped from the front without a scan.
    order: VecDeque<(Timestamp, DeduplicationKey)>,
}

impl Deduplication {
    fn forget_before(&mut self, now: Timestamp) {
        while let Some((_, key)) = self
            .order
            .pop_front_if(|(at, _)| now >= *at + DEDUPLICATION_WINDOW)
        {
            self.recent.remove(&key);
        }
    }

    fn find(&mut self, key: &DeduplicationKey, now: Timestamp) -> Option<Sent> {
        self.forget_before(now);
        self.recent.get(key).copied()
    }

    fn record(&mut self, key: DeduplicationKey, at: Timestamp, sent: Sent) {
        self.order.push_back((at, key.clone()));
        self.recent.insert(key, sent);
    }
}

/// A message as `ReceiveMessage` reports it.
#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Delivery {
    message_id: MessageId,
    receipt_handle: ReceiptHandle,
    body: String,
    receive_count: u32,
    sent_at: Timestamp,
    first_received_at: Timestamp,
    #[serde(skip_serializing_if = "Option::is_none")]
    group_id: Option<GroupId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sequence_number: Option<SequenceNumber>,
}

/// What one `ReceiveMessage` produced.
#[derive(Debug)]
pub struct Received {
    pub deliveries: Vec<Delivery>,
    /// Present when the queue has a dead-letter configuration; its messages
    /// may be empty.
    pub dead_letters: Option<DeadLetters>,
}

/// Messages taken out of a queue for its dead-letter queue, in move order.
#[derive(Debug)]
pub struct DeadLetters {
    pub queue: QueueName,
    pub messages: Vec<Message>,
}

#[derive(Debug)]
pub struct Queue {
    attributes: QueueAttributes,
    messages: BTreeMap<Arrival, Message>,
    arrivals: Counter,
    /// `(sent_at, arrival)` of every message, so retention can remove the
    /// oldest ones first; arrival order is not send order once dead-letter
    /// moves append old messages.
    by_age: BTreeSet<(Timestamp, Arrival)>,
    /// The latest receipt handle of every message that has been delivered.
    handles: HashMap<ReceiptHandle, Arrival>,
    sequences: Counter,
    deduplication: Deduplication,
}

impl Queue {
    pub fn new(attributes: QueueAttributes) -> Self {
        Queue {
            attributes,
            messages: BTreeMap::new(),
            arrivals: Counter::default(),
            by_age: BTreeSet::new(),
            handles: HashMap::new(),
            sequences: Counter::default(),
            deduplication: Deduplication::default(),
        }
    }

    pub fn attributes(&self) -> &QueueAttributes {
        &self.attributes
    }

    pub fn attributes_mut(&mut self) -> &mut QueueAttributes {
        &mut self.attributes
    }

    pub fn is_fifo(&self) -> bool {
        self.attributes.kind.is_fifo()
    }

    pub fn counts(&self, now: Timestamp) -> Counts {
        self.messages
            .values()
            .fold(Counts::default(), |mut counts, message| {
                match message.state(now) {
                    MessageState::Visible => counts.visible += 1,
                    MessageState::InFlight => counts.in_flight += 1,
                    MessageState::Delayed => counts.delayed += 1,
                }
                counts
            })
    }

    /// Removes every message whose retention period has ended by `now`.
    pub fn expire(&mut self, now: Timestamp) {
        while let Some(&(sent_at, arrival)) = self.by_age.first() {
            if now < sent_at + self.attributes.retention {
                break;
            }
            self.remove(arrival);
        }
    }

    /// Removes every message; sequence numbers and deduplication carry on.
    pub fn purge(&mut self) {
        self.messages.clear();
        self.by_age.clear();
        self.handles.clear();
    }

    /// Applies the `SendMessage` rules that depend on the queue, then accepts
    /// the message with an id from `message_ids`.
    pub fn send(
        &mut self,
        message: NewMessage,
        now: Timestamp,
        message_ids: &mut Counter,
    ) -> Result<Sent, Error> {
        let NewMessage {
            body,
            delay,
            group,
            deduplication_id,
        } = message;
        let fifo = match self.attributes.kind {
            QueueKind::Standard => {
                if group.is_some() {
                    return Err(Error::FifoParameterOnStandard("GroupId"));
                }
                if deduplication_id.is_some() {
                    return Err(Error::FifoParameterOnStandard("DeduplicationId"));
                }
                None
            }
            QueueKind::Fifo {
                content_based_deduplication,
            } => {
                if delay.is_some() {
                    return Err(Error::DelayOnFifo);
                }
                let group = group.ok_or(Error::MissingParameter("GroupId"))?;
                let key = match deduplication_id {
                    Some(key) => key,
                    None if content_based_deduplication => DeduplicationKey::of_body(&body),
                    None => return Err(Error::MissingParameter("DeduplicationId")),
                };
                if let Some(original) = self.deduplication.find(&key, now) {
                    return Ok(original);
                }
                Some((group, key))
            }
        };
        if self.messages.len() >= MAX_MESSAGES {
            return Err(Error::QueueFull);
        }
        let id = MessageId(message_ids.next());
        let until = now + delay.unwrap_or(self.attributes.delay);
        let fifo = fifo.map(|(group, key)| {
            let sequence = SequenceNumber(self.sequences.next());
            let sent = Sent {
                id,
                sequence: Some(sequence),
            };
            self.deduplication.record(key, now, sent);
            FifoTag { group, sequence }
        });
        let sent = Sent {
            id,
            sequence: fifo.as_ref().map(|tag| tag.sequence),
        };
        self.push(Message {
            id,
            body,
            sent_at: now,
            receive_count: 0,
            first_received_at: None,
            fifo,
            hold: Hold::Arrived { until },
        });
        Ok(sent)
    }

    /// Delivers up to `max_messages` eligible messages in arrival order,
    /// drawing handles from `receipts`. Eligible messages that reached the
    /// dead-letter threshold are taken out instead, for the caller to hand to
    /// the dead-letter queue.
    pub fn receive(
        &mut self,
        max_messages: u32,
        visibility_timeout: Option<Seconds>,
        now: Timestamp,
        receipts: &mut Counter,
    ) -> Received {
        let timeout = visibility_timeout.unwrap_or(self.attributes.visibility_timeout);
        let Queue {
            attributes,
            messages,
            handles,
            ..
        } = self;
        let mut remaining = max_messages;
        let mut deliveries = Vec::new();
        let mut moving = Vec::new();
        // Groups that already have a message earlier in the queue; later
        // messages of those groups are not eligible.
        let mut blocked = HashSet::new();
        for (&arrival, message) in messages.iter_mut() {
            if remaining == 0 {
                break;
            }
            let group_free = message
                .fifo
                .as_ref()
                .is_none_or(|tag| !blocked.contains(&tag.group));
            if group_free && message.state(now) == MessageState::Visible {
                if attributes.dead_letter.as_ref().is_some_and(|dead_letter| {
                    message.receive_count >= dead_letter.max_receive_count
                }) {
                    // It leaves the queue, so it neither counts toward
                    // `max_messages` nor blocks its group.
                    moving.push(arrival);
                    continue;
                }
                let handle = ReceiptHandle(receipts.next());
                if let Some(previous) = message.handle() {
                    handles.remove(&previous);
                }
                handles.insert(handle, arrival);
                message.receive_count += 1;
                let first_received_at = *message.first_received_at.get_or_insert(now);
                message.hold = Hold::Delivered {
                    until: now + timeout,
                    handle,
                };
                deliveries.push(Delivery {
                    message_id: message.id,
                    receipt_handle: handle,
                    body: message.body.clone(),
                    receive_count: message.receive_count,
                    sent_at: message.sent_at,
                    first_received_at,
                    group_id: message.fifo.as_ref().map(|tag| tag.group.clone()),
                    sequence_number: message.fifo.as_ref().map(|tag| tag.sequence),
                });
                remaining -= 1;
            }
            // A shared reborrow lives as long as the walk, so `blocked` can
            // keep pointing at this message's group.
            let message = &*message;
            if let Some(tag) = &message.fifo {
                blocked.insert(&tag.group);
            }
        }
        let messages = moving
            .into_iter()
            .map(|arrival| self.remove(arrival))
            .collect();
        let dead_letters = self
            .attributes
            .dead_letter
            .as_ref()
            .map(|dead_letter| DeadLetters {
                queue: dead_letter.queue.clone(),
                messages,
            });
        Received {
            deliveries,
            dead_letters,
        }
    }

    /// Appends messages moved from another queue: they start over as never
    /// received and visible, keep their `SentAt` for retention, and bypass
    /// deduplication and the message limit.
    pub fn accept_dead_letters(&mut self, messages: Vec<Message>, now: Timestamp) {
        for mut message in messages {
            message.receive_count = 0;
            message.first_received_at = None;
            message.hold = Hold::Arrived { until: now };
            message.fifo = message.fifo.map(|tag| FifoTag {
                group: tag.group,
                sequence: SequenceNumber(self.sequences.next()),
            });
            self.push(message);
        }
        self.expire(now);
    }

    pub fn delete(&mut self, handle: ReceiptHandle, now: Timestamp) -> Result<(), Error> {
        let arrival = self.in_flight(handle, now)?;
        self.remove(arrival);
        Ok(())
    }

    pub fn change_visibility(
        &mut self,
        handle: ReceiptHandle,
        timeout: Seconds,
        now: Timestamp,
    ) -> Result<(), Error> {
        let arrival = self.in_flight(handle, now)?;
        let message = self.message_mut(arrival);
        message.hold = Hold::Delivered {
            until: now + timeout,
            handle,
        };
        Ok(())
    }

    /// The message that `handle` currently holds in flight.
    fn in_flight(&self, handle: ReceiptHandle, now: Timestamp) -> Result<Arrival, Error> {
        self.handles
            .get(&handle)
            .copied()
            .filter(|&arrival| self.message(arrival).state(now) == MessageState::InFlight)
            .ok_or(Error::ReceiptHandleExpired(handle))
    }

    fn message(&self, arrival: Arrival) -> &Message {
        self.messages
            .get(&arrival)
            .expect("every indexed arrival refers to a held message")
    }

    fn message_mut(&mut self, arrival: Arrival) -> &mut Message {
        self.messages
            .get_mut(&arrival)
            .expect("every indexed arrival refers to a held message")
    }

    fn push(&mut self, message: Message) {
        let arrival = Arrival(self.arrivals.next());
        self.by_age.insert((message.sent_at, arrival));
        self.messages.insert(arrival, message);
    }

    fn remove(&mut self, arrival: Arrival) -> Message {
        let message = self
            .messages
            .remove(&arrival)
            .expect("every indexed arrival refers to a held message");
        self.by_age.remove(&(message.sent_at, arrival));
        if let Some(handle) = message.handle() {
            self.handles.remove(&handle);
        }
        message
    }
}
