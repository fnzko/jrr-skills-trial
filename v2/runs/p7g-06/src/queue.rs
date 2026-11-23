//! A single queue: its settings, its messages and the rules that act on them.

use std::collections::{BTreeMap, HashMap, HashSet};

use serde::Serialize;

use crate::error::{Error, ErrorCode};
use crate::handles::{HandleLog, Issued};
use crate::request::{AttributeSet, SendRequest};
use crate::response::{ReceivedMessage, SentMessage};
use crate::types::{
    ArrivalKey, Body, GroupId, Instant, MessageId, QueueId, QueueName, ReceiptHandle, Seconds,
};

pub const MAX_QUEUES: usize = 1000;
pub const MAX_MESSAGES: usize = 100_000;

const DEFAULT_VISIBILITY_TIMEOUT: Seconds = Seconds::new(30);
const DEFAULT_DELAY: Seconds = Seconds::new(0);
const DEFAULT_RETENTION: Seconds = Seconds::new(345_600);

/// A send is a duplicate while less than this long has passed since the original.
const DEDUPLICATION_WINDOW: Seconds = Seconds::new(300);

/// The type of a queue, which never changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Standard,
    Fifo { content_based_deduplication: bool },
}

impl Kind {
    pub fn is_fifo(self) -> bool {
        matches!(self, Kind::Fifo { .. })
    }

    pub fn content_based_deduplication(self) -> bool {
        matches!(
            self,
            Kind::Fifo {
                content_based_deduplication: true
            }
        )
    }
}

/// Where a message goes when it has been received too often.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeadLetter {
    pub queue: QueueName,
    pub max_receive_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub visibility_timeout: Seconds,
    pub delay: Seconds,
    pub retention: Seconds,
    pub kind: Kind,
    pub dead_letter: Option<DeadLetter>,
}

impl Settings {
    pub fn defaults(fifo: bool) -> Self {
        Self {
            visibility_timeout: DEFAULT_VISIBILITY_TIMEOUT,
            delay: DEFAULT_DELAY,
            retention: DEFAULT_RETENTION,
            kind: if fifo {
                Kind::Fifo {
                    content_based_deduplication: false,
                }
            } else {
                Kind::Standard
            },
            dead_letter: None,
        }
    }

    /// Takes the attributes present in `attributes`; the dead-letter configuration is
    /// replaced only when `dead_letter` is given.
    pub fn update(&mut self, attributes: &AttributeSet<'_>, dead_letter: Option<DeadLetter>) {
        if let Some(seconds) = attributes.visibility_timeout {
            self.visibility_timeout = seconds;
        }
        if let Some(seconds) = attributes.delay {
            self.delay = seconds;
        }
        if let Some(seconds) = attributes.retention {
            self.retention = seconds;
        }
        if let (Kind::Fifo { .. }, Some(enabled)) =
            (self.kind, attributes.content_based_deduplication)
        {
            self.kind = Kind::Fifo {
                content_based_deduplication: enabled,
            };
        }
        if dead_letter.is_some() {
            self.dead_letter = dead_letter;
        }
    }

    /// True if creating a queue with `attributes` would give exactly these settings.
    pub fn matches(&self, attributes: &AttributeSet<'_>) -> bool {
        let dead_letter = match (
            &self.dead_letter,
            attributes.dead_letter_queue,
            attributes.max_receive_count,
        ) {
            (None, None, None) => true,
            (Some(own), Some(queue), Some(count)) => {
                own.queue.as_str() == queue && own.max_receive_count == count
            }
            _ => false,
        };
        dead_letter
            && self.visibility_timeout
                == attributes
                    .visibility_timeout
                    .unwrap_or(DEFAULT_VISIBILITY_TIMEOUT)
            && self.delay == attributes.delay.unwrap_or(DEFAULT_DELAY)
            && self.retention == attributes.retention.unwrap_or(DEFAULT_RETENTION)
            && self.kind.is_fifo() == attributes.fifo.unwrap_or(false)
            && self.kind.content_based_deduplication()
                == attributes.content_based_deduplication.unwrap_or(false)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Visible,
    InFlight,
    Delayed,
}

/// The message counts per state of `GetQueueAttributes`.
#[derive(Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Counts {
    pub visible: usize,
    pub in_flight: usize,
    pub delayed: usize,
}

#[derive(Debug)]
struct FifoPlacement {
    group: GroupId,
    sequence: u64,
}

/// The latest delivery of a message.
#[derive(Debug, Clone, Copy)]
struct Delivery {
    handle: ReceiptHandle,
    /// The message is in flight before this instant.
    visible_at: Instant,
}

#[derive(Debug)]
pub struct Message {
    id: MessageId,
    body: Body,
    sent_at: Instant,
    /// Not visible before this instant.
    delayed_until: Instant,
    receive_count: u32,
    first_received_at: Option<Instant>,
    delivery: Option<Delivery>,
    fifo: Option<FifoPlacement>,
}

impl Message {
    fn state(&self, now: Instant) -> State {
        if now < self.delayed_until {
            State::Delayed
        } else if self.delivery.is_some_and(|d| now < d.visible_at) {
            State::InFlight
        } else {
            State::Visible
        }
    }
}

/// Messages moved out of a queue, to be appended to its dead-letter queue.
#[derive(Debug)]
pub struct DeadLettered {
    pub queue: QueueName,
    pub messages: Vec<Message>,
}

/// What `Queue::receive` did.
#[derive(Debug)]
pub struct ReceiveOutcome {
    pub delivered: Vec<ReceivedMessage>,
    pub dead_lettered: Option<DeadLettered>,
}

#[derive(Debug, Clone, Copy)]
enum Verdict {
    Deliver,
    DeadLetter,
}

/// An accepted send, remembered so that repeats within the window can be recognized.
#[derive(Debug)]
struct Accepted {
    at: Instant,
    sent: SentMessage,
}

/// What `Queue::send` did with a message.
#[derive(Debug)]
pub enum SendOutcome {
    Added(SentMessage),
    Duplicate(SentMessage),
}

#[derive(Debug)]
pub struct Queue {
    id: QueueId,
    pub settings: Settings,
    /// Messages by arrival order; the key is the position the message arrived in.
    messages: BTreeMap<ArrivalKey, Message>,
    next_key: ArrivalKey,
    next_sequence: u64,
    /// A lower bound of the `sent_at` of all messages, so that nothing is scanned
    /// while no message can have outlived the retention.
    oldest_sent: Option<Instant>,
    deduplication: HashMap<String, Accepted>,
}

impl Queue {
    pub fn new(id: QueueId, settings: Settings) -> Self {
        Self {
            id,
            settings,
            messages: BTreeMap::new(),
            next_key: ArrivalKey::FIRST,
            next_sequence: 1,
            oldest_sent: None,
            deduplication: HashMap::new(),
        }
    }

    /// Removes the messages that outlived the retention; call before looking at the queue.
    pub fn expire(&mut self, now: Instant) {
        let retention = self.settings.retention;
        match self.oldest_sent {
            Some(oldest) if now >= oldest.plus(retention) => {}
            _ => return,
        }
        self.messages
            .retain(|_, message| now < message.sent_at.plus(retention));
        self.oldest_sent = self.messages.values().map(|m| m.sent_at).min();
    }

    pub fn counts(&self, now: Instant) -> Counts {
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

    /// Drops every message. Sequence numbers and deduplication carry on.
    pub fn purge(&mut self) {
        self.messages.clear();
        self.oldest_sent = None;
    }

    /// Applies the rules of `SendMessage` after the queue lookup. `id` is used only
    /// if the message is added.
    pub fn send(
        &mut self,
        request: SendRequest<'_>,
        id: MessageId,
        now: Instant,
    ) -> Result<SendOutcome, Error> {
        let SendRequest {
            body,
            delay,
            group,
            deduplication_id,
            ..
        } = request;
        let (placement_group, deduplication_key) = match self.settings.kind {
            Kind::Standard => {
                if group.is_some() || deduplication_id.is_some() {
                    return Err(unsupported("GroupId and DeduplicationId need a FIFO queue"));
                }
                (None, None)
            }
            Kind::Fifo {
                content_based_deduplication,
            } => {
                if delay.is_some() {
                    return Err(unsupported("DelaySeconds is not allowed on a FIFO queue"));
                }
                let Some(group) = group else {
                    return Err(Error::missing("GroupId"));
                };
                if deduplication_id.is_none() && !content_based_deduplication {
                    return Err(Error::missing("DeduplicationId"));
                }
                let key = deduplication_id
                    .as_ref()
                    .map_or(body.as_str(), |id| id.as_str());
                let original = self
                    .deduplication
                    .get(key)
                    .filter(|accepted| now < accepted.at.plus(DEDUPLICATION_WINDOW));
                if let Some(original) = original {
                    return Ok(SendOutcome::Duplicate(original.sent));
                }
                let key = deduplication_id
                    .map_or_else(|| body.as_str().to_owned(), |id| id.into_string());
                (Some(group), Some(key))
            }
        };
        if self.messages.len() >= MAX_MESSAGES {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                format!("the queue already holds {MAX_MESSAGES} messages"),
            ));
        }
        let delay = delay.unwrap_or(self.settings.delay);
        let fifo = placement_group.map(|group| FifoPlacement {
            group,
            sequence: self.take_sequence(),
        });
        let sent = SentMessage {
            message_id: id,
            sequence_number: fifo.as_ref().map(|placement| placement.sequence),
        };
        if let Some(key) = deduplication_key {
            self.deduplication.insert(key, Accepted { at: now, sent });
        }
        self.push(Message {
            id,
            body,
            sent_at: now,
            delayed_until: now.plus(delay),
            receive_count: 0,
            first_received_at: None,
            delivery: None,
            fifo,
        });
        Ok(SendOutcome::Added(sent))
    }

    fn take_sequence(&mut self) -> u64 {
        let sequence = self.next_sequence;
        self.next_sequence += 1;
        sequence
    }

    fn push(&mut self, message: Message) {
        self.oldest_sent = Some(
            self.oldest_sent
                .map_or(message.sent_at, |oldest| oldest.min(message.sent_at)),
        );
        self.messages.insert(self.next_key, message);
        self.next_key = self.next_key.next();
    }

    /// Appends a message that another queue gave up on, as a fresh message of this queue.
    pub fn accept_dead_letter(&mut self, mut message: Message, now: Instant) {
        message.receive_count = 0;
        message.first_received_at = None;
        message.delivery = None;
        message.delayed_until = message.sent_at;
        if let Some(placement) = &mut message.fifo {
            placement.sequence = self.take_sequence();
        }
        // Retention counts from the original send, so the message may be too old already.
        if now < message.sent_at.plus(self.settings.retention) {
            self.push(message);
        }
    }

    /// Delivers up to `max_messages` eligible messages in arrival order and takes out the
    /// ones that were received too often.
    pub fn receive(
        &mut self,
        max_messages: u32,
        visibility_timeout: Seconds,
        now: Instant,
        handles: &mut HandleLog,
    ) -> ReceiveOutcome {
        let dead_letter = self.settings.dead_letter.as_ref();
        let mut plan = Vec::new();
        let mut remaining = max_messages;
        // Groups that still have an earlier message in the queue.
        let mut occupied: HashSet<&GroupId> = HashSet::new();
        for (key, message) in &self.messages {
            if remaining == 0 {
                break;
            }
            let group = message.fifo.as_ref().map(|placement| &placement.group);
            if group.is_some_and(|group| occupied.contains(group)) {
                continue;
            }
            if message.state(now) != State::Visible {
                occupied.extend(group);
                continue;
            }
            match dead_letter {
                Some(config) if message.receive_count >= config.max_receive_count => {
                    plan.push((*key, Verdict::DeadLetter));
                }
                _ => {
                    plan.push((*key, Verdict::Deliver));
                    remaining -= 1;
                    occupied.extend(group);
                }
            }
        }

        let mut delivered = Vec::new();
        let mut moved = Vec::new();
        for (key, verdict) in plan {
            match verdict {
                Verdict::Deliver => {
                    let message = self
                        .messages
                        .get_mut(&key)
                        .expect("the plan only holds keys read from the map");
                    message.receive_count += 1;
                    let first_received_at = *message.first_received_at.get_or_insert(now);
                    let handle = handles.issue(self.id, key);
                    message.delivery = Some(Delivery {
                        handle,
                        visible_at: now.plus(visibility_timeout),
                    });
                    delivered.push(ReceivedMessage {
                        message_id: message.id,
                        receipt_handle: handle,
                        body: message.body.clone(),
                        receive_count: message.receive_count,
                        sent_at: message.sent_at,
                        first_received_at,
                        group_id: message.fifo.as_ref().map(|p| p.group.clone()),
                        sequence_number: message.fifo.as_ref().map(|p| p.sequence),
                    });
                }
                Verdict::DeadLetter => moved.extend(self.messages.remove(&key)),
            }
        }
        let dead_lettered = dead_letter
            .filter(|_| !moved.is_empty())
            .map(|config| DeadLettered {
                queue: config.queue.clone(),
                messages: moved,
            });
        ReceiveOutcome {
            delivered,
            dead_lettered,
        }
    }

    /// The delivery `issued` names, if it is the latest of a message in flight in this queue.
    fn current_delivery(&mut self, issued: Issued, now: Instant) -> Result<&mut Delivery, Error> {
        if issued.queue != self.id {
            return Err(expired("the receipt handle belongs to another queue"));
        }
        self.messages
            .get_mut(&issued.key)
            .and_then(|message| message.delivery.as_mut())
            .filter(|delivery| delivery.handle == issued.handle && now < delivery.visible_at)
            .ok_or_else(|| expired("the message is gone, delivered again or visible"))
    }

    pub fn delete_message(&mut self, issued: Issued, now: Instant) -> Result<(), Error> {
        self.current_delivery(issued, now)?;
        self.messages.remove(&issued.key);
        Ok(())
    }

    pub fn change_visibility(
        &mut self,
        issued: Issued,
        timeout: Seconds,
        now: Instant,
    ) -> Result<(), Error> {
        self.current_delivery(issued, now)?.visible_at = now.plus(timeout);
        Ok(())
    }
}

fn expired(message: &str) -> Error {
    Error::new(ErrorCode::ReceiptHandleExpired, message)
}

fn unsupported(message: &str) -> Error {
    Error::new(ErrorCode::UnsupportedOperation, message)
}
