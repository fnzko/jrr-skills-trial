//! One queue: its settings, its messages in arrival order and the state-machine rules of
//! sections 4 and 6 of the specification.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use serde::Serialize;

use crate::command::{Attributes, DeadLetterRequest, SendMessage};
use crate::error::{CommandError, ErrorCode};
use crate::ids::{DedupKey, GroupId, MessageId, QueueName, ReceiptHandle, SequenceNumber};
use crate::reply::{Counts, Received, ReceivedMessage, Sent};
use crate::time::{Seconds, Timestamp};

pub const MAX_MESSAGES_PER_QUEUE: usize = 100_000;
pub const DEFAULT_VISIBILITY_TIMEOUT: Seconds = Seconds(30);
pub const DEFAULT_DELAY: Seconds = Seconds(0);
pub const DEFAULT_RETENTION: Seconds = Seconds(345_600);

/// A FIFO queue treats a second send with the same deduplication id as a duplicate for
/// this long after the send that was accepted.
const DEDUP_WINDOW: Seconds = Seconds(300);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueType {
    Standard,
    Fifo { content_based_deduplication: bool },
}

impl QueueType {
    pub fn is_fifo(self) -> bool {
        matches!(self, Self::Fifo { .. })
    }

    pub fn content_based_deduplication(self) -> bool {
        matches!(
            self,
            Self::Fifo {
                content_based_deduplication: true
            }
        )
    }

    fn requested(attributes: &Attributes<'_>) -> Self {
        if attributes.fifo.unwrap_or(false) {
            Self::Fifo {
                content_based_deduplication: attributes
                    .content_based_deduplication
                    .unwrap_or(false),
            }
        } else {
            Self::Standard
        }
    }
}

/// Where messages go after too many deliveries. Both members are set or neither is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DeadLetter {
    #[serde(rename = "DeadLetterQueue")]
    pub queue: QueueName,
    #[serde(rename = "MaxReceiveCount")]
    pub max_receive_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueSettings {
    pub visibility_timeout: Seconds,
    pub delay: Seconds,
    pub retention: Seconds,
    pub queue_type: QueueType,
    pub dead_letter: Option<DeadLetter>,
}

impl QueueSettings {
    /// Settings for a new queue: the attributes given, the defaults for the rest.
    pub fn requested(attributes: &Attributes<'_>, dead_letter: Option<DeadLetter>) -> Self {
        Self {
            visibility_timeout: attributes
                .visibility_timeout
                .unwrap_or(DEFAULT_VISIBILITY_TIMEOUT),
            delay: attributes.delay.unwrap_or(DEFAULT_DELAY),
            retention: attributes.retention.unwrap_or(DEFAULT_RETENTION),
            queue_type: QueueType::requested(attributes),
            dead_letter,
        }
    }

    /// Whether a `CreateQueue` request describes exactly these settings. The dead-letter
    /// target is compared by name because the requested one need not exist.
    fn matches(
        &self,
        attributes: &Attributes<'_>,
        dead_letter: Option<DeadLetterRequest<'_>>,
    ) -> bool {
        let current = self
            .dead_letter
            .as_ref()
            .map(|d| (d.queue.as_str(), d.max_receive_count));
        let requested = dead_letter.map(|d| (d.queue, d.max_receive_count));
        self.visibility_timeout
            == attributes
                .visibility_timeout
                .unwrap_or(DEFAULT_VISIBILITY_TIMEOUT)
            && self.delay == attributes.delay.unwrap_or(DEFAULT_DELAY)
            && self.retention == attributes.retention.unwrap_or(DEFAULT_RETENTION)
            && self.queue_type == QueueType::requested(attributes)
            && current == requested
    }
}

/// Position of a message among everything ever added to its queue; never reused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Arrival(u64);

/// Why a message is not (yet) visible. Whether the reason still applies depends on the clock.
#[derive(Debug, Clone, Copy)]
enum Hold {
    Free,
    Delayed {
        until: Timestamp,
    },
    InFlight {
        until: Timestamp,
        handle: ReceiptHandle,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MessageState {
    Visible,
    InFlight,
    Delayed,
}

#[derive(Debug, Clone, Copy)]
enum Deliveries {
    Never,
    Some { count: u32, first_at: Timestamp },
}

#[derive(Debug)]
struct Message {
    id: MessageId,
    body: String,
    sent_at: Timestamp,
    deliveries: Deliveries,
    group: Option<GroupId>,
    sequence: Option<SequenceNumber>,
    hold: Hold,
}

impl Message {
    fn state(&self, now: Timestamp) -> MessageState {
        match self.hold {
            Hold::Delayed { until } if now < until => MessageState::Delayed,
            Hold::InFlight { until, .. } if now < until => MessageState::InFlight,
            _ => MessageState::Visible,
        }
    }

    fn receive_count(&self) -> u32 {
        match self.deliveries {
            Deliveries::Never => 0,
            Deliveries::Some { count, .. } => count,
        }
    }

    /// Records a delivery and returns the time of the first one.
    fn deliver(
        &mut self,
        now: Timestamp,
        visibility_timeout: Seconds,
        handle: ReceiptHandle,
    ) -> Timestamp {
        let (count, first_at) = match self.deliveries {
            Deliveries::Never => (1, now),
            Deliveries::Some { count, first_at } => (count.saturating_add(1), first_at),
        };
        self.deliveries = Deliveries::Some { count, first_at };
        self.hold = Hold::InFlight {
            until: now + visibility_timeout,
            handle,
        };
        first_at
    }
}

/// A message about to be added to a queue, either sent or moved from another queue.
#[derive(Debug)]
pub struct NewMessage {
    id: MessageId,
    body: String,
    sent_at: Timestamp,
    group: Option<GroupId>,
    delayed_until: Option<Timestamp>,
}

/// Messages a `ReceiveMessage` took out of a queue for its dead-letter queue.
#[derive(Debug)]
pub struct Transfer {
    pub to: QueueName,
    pub messages: Vec<NewMessage>,
}

#[derive(Debug)]
pub struct Receipt {
    pub received: Received,
    pub dead_lettered: Option<Transfer>,
}

/// A send that was accepted at `at`, remembered to recognize duplicates.
#[derive(Debug)]
struct AcceptedSend {
    at: Timestamp,
    result: Sent,
}

#[derive(Debug)]
pub struct Queue {
    settings: QueueSettings,
    messages: BTreeMap<Arrival, Message>,
    next_arrival: u64,
    /// Orders messages by send time so retention only looks at the oldest ones; it holds
    /// exactly the messages of `messages`.
    by_sent_at: BTreeSet<(Timestamp, Arrival)>,
    /// The latest receipt handle of every message that has been delivered.
    handles: HashMap<ReceiptHandle, Arrival>,
    next_sequence: SequenceNumber,
    accepted: HashMap<DedupKey, AcceptedSend>,
}

impl Queue {
    pub fn new(settings: QueueSettings) -> Self {
        Self {
            settings,
            messages: BTreeMap::new(),
            next_arrival: 0,
            by_sent_at: BTreeSet::new(),
            handles: HashMap::new(),
            next_sequence: SequenceNumber::FIRST,
            accepted: HashMap::new(),
        }
    }

    pub fn settings(&self) -> &QueueSettings {
        &self.settings
    }

    pub fn matches_request(
        &self,
        attributes: &Attributes<'_>,
        dead_letter: Option<DeadLetterRequest<'_>>,
    ) -> bool {
        self.settings.matches(attributes, dead_letter)
    }

    /// Removes the dead-letter configuration if it points at `target`.
    pub fn forget_dead_letter_queue(&mut self, target: &str) {
        self.settings
            .dead_letter
            .take_if(|d| d.queue.as_str() == target);
    }

    /// Applies the attributes present; the others keep their values.
    pub fn update(
        &mut self,
        attributes: &Attributes<'_>,
        dead_letter: Option<DeadLetter>,
        now: Timestamp,
    ) {
        let settings = &mut self.settings;
        if let Some(value) = attributes.visibility_timeout {
            settings.visibility_timeout = value;
        }
        if let Some(value) = attributes.delay {
            settings.delay = value;
        }
        if let Some(value) = attributes.retention {
            settings.retention = value;
        }
        if let QueueType::Fifo {
            content_based_deduplication,
        } = &mut settings.queue_type
            && let Some(value) = attributes.content_based_deduplication
        {
            *content_based_deduplication = value;
        }
        if dead_letter.is_some() {
            settings.dead_letter = dead_letter;
        }
        self.sweep(now);
    }

    /// Drops every message whose retention has run out. Called before a queue is used, so
    /// the effect is the same as removing them at the moment they expire.
    pub fn sweep(&mut self, now: Timestamp) {
        while let Some(&(sent_at, arrival)) = self.by_sent_at.first()
            && sent_at + self.settings.retention <= now
        {
            self.remove(arrival);
        }
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

    /// Removes every message. Sequence numbers and deduplication carry on.
    pub fn purge(&mut self) {
        self.messages.clear();
        self.by_sent_at.clear();
        self.handles.clear();
    }

    fn remove(&mut self, arrival: Arrival) -> Option<Message> {
        let message = self.messages.remove(&arrival)?;
        self.by_sent_at.remove(&(message.sent_at, arrival));
        if let Hold::InFlight { handle, .. } = message.hold {
            self.handles.remove(&handle);
        }
        Some(message)
    }

    /// Adds a message at the end and returns its sequence number on a FIFO queue. The
    /// message limit is the caller's business: dead-letter moves ignore it.
    pub fn append(&mut self, new: NewMessage) -> Option<SequenceNumber> {
        let arrival = Arrival(self.next_arrival);
        self.next_arrival += 1;
        let sequence = self
            .settings
            .queue_type
            .is_fifo()
            .then(|| self.next_sequence.advance());
        self.by_sent_at.insert((new.sent_at, arrival));
        self.messages.insert(
            arrival,
            Message {
                id: new.id,
                body: new.body,
                sent_at: new.sent_at,
                deliveries: Deliveries::Never,
                group: new.group,
                sequence,
                hold: new
                    .delayed_until
                    .map_or(Hold::Free, |until| Hold::Delayed { until }),
            },
        );
        sequence
    }

    pub fn send(
        &mut self,
        request: SendMessage<'_>,
        now: Timestamp,
        next_id: &mut MessageId,
    ) -> Result<Sent, CommandError> {
        let dedup_key = match self.settings.queue_type {
            QueueType::Standard => {
                if request.group.is_some() || request.deduplication_id.is_some() {
                    return Err(CommandError::new(
                        ErrorCode::UnsupportedOperation,
                        "GroupId and DeduplicationId are only supported on FIFO queues",
                    ));
                }
                None
            }
            QueueType::Fifo {
                content_based_deduplication,
            } => {
                if request.delay.is_some() {
                    return Err(CommandError::new(
                        ErrorCode::UnsupportedOperation,
                        "DelaySeconds is not supported per message on FIFO queues",
                    ));
                }
                if request.group.is_none() {
                    return Err(CommandError::missing("GroupId"));
                }
                let key = match request.deduplication_id {
                    Some(key) => key,
                    None if content_based_deduplication => DedupKey::from_body(request.body),
                    None => return Err(CommandError::missing("DeduplicationId")),
                };
                if let Some(earlier) = self.accepted.get(&key)
                    && now < earlier.at + DEDUP_WINDOW
                {
                    return Ok(earlier.result);
                }
                Some(key)
            }
        };
        if self.messages.len() >= MAX_MESSAGES_PER_QUEUE {
            return Err(CommandError::new(
                ErrorCode::LimitExceeded,
                format!("queue already holds {MAX_MESSAGES_PER_QUEUE} messages"),
            ));
        }
        let delay = request.delay.unwrap_or(self.settings.delay);
        let message_id = next_id.advance();
        let sequence_number = self.append(NewMessage {
            id: message_id,
            body: request.body.to_owned(),
            sent_at: now,
            group: request.group,
            delayed_until: (delay > Seconds(0)).then(|| now + delay),
        });
        let sent = Sent {
            message_id,
            sequence_number,
        };
        if let Some(key) = dedup_key {
            self.accepted.insert(
                key,
                AcceptedSend {
                    at: now,
                    result: sent,
                },
            );
        }
        Ok(sent)
    }

    /// Delivers up to `max_messages` messages, moving the ones that were delivered too
    /// often out of the queue (see `Receipt::dead_lettered`).
    pub fn receive(
        &mut self,
        max_messages: usize,
        visibility_timeout: Option<Seconds>,
        now: Timestamp,
        next_handle: &mut ReceiptHandle,
    ) -> Receipt {
        let visibility_timeout = visibility_timeout.unwrap_or(self.settings.visibility_timeout);
        let max_receive_count = self
            .settings
            .dead_letter
            .as_ref()
            .map(|d| d.max_receive_count);
        let mut received = Vec::new();
        let mut dead = Vec::new();
        // Groups with an earlier message that is still in the queue, in any state.
        let mut earlier_groups: HashSet<&GroupId> = HashSet::new();
        for (&arrival, message) in &mut self.messages {
            if received.len() == max_messages {
                break;
            }
            let blocked = message
                .group
                .as_ref()
                .is_some_and(|group| earlier_groups.contains(group));
            if !blocked && message.state(now) == MessageState::Visible {
                if max_receive_count.is_some_and(|max| message.receive_count() >= max) {
                    dead.push(arrival);
                    continue;
                }
                let handle = next_handle.advance();
                if let Hold::InFlight {
                    handle: previous, ..
                } = message.hold
                {
                    self.handles.remove(&previous);
                }
                self.handles.insert(handle, arrival);
                let first_at = message.deliver(now, visibility_timeout, handle);
                received.push(ReceivedMessage {
                    message_id: message.id,
                    receipt_handle: handle,
                    body: message.body.clone(),
                    receive_count: message.receive_count(),
                    sent_at: message.sent_at,
                    first_received_at: first_at,
                    group_id: message.group.clone(),
                    sequence_number: message.sequence,
                });
            }
            let message: &Message = message;
            if let Some(group) = &message.group {
                earlier_groups.insert(group);
            }
        }
        let moved: Vec<NewMessage> = dead
            .into_iter()
            .filter_map(|arrival| self.remove(arrival))
            .map(|message| NewMessage {
                id: message.id,
                body: message.body,
                sent_at: message.sent_at,
                group: message.group,
                delayed_until: None,
            })
            .collect();
        let dead_lettered = match &self.settings.dead_letter {
            Some(config) if !moved.is_empty() => Some(Transfer {
                to: config.queue.clone(),
                messages: moved,
            }),
            _ => None,
        };
        Receipt {
            received: Received { messages: received },
            dead_lettered,
        }
    }

    /// The message whose latest handle is `handle`, provided it is in flight at `now`.
    fn in_flight(
        &mut self,
        handle: ReceiptHandle,
        now: Timestamp,
    ) -> Result<(Arrival, &mut Message), CommandError> {
        let expired = || {
            CommandError::new(
                ErrorCode::ReceiptHandleExpired,
                format!("receipt handle {handle} is no longer usable in this queue"),
            )
        };
        let arrival = *self.handles.get(&handle).ok_or_else(expired)?;
        let message = self.messages.get_mut(&arrival).ok_or_else(expired)?;
        match message.hold {
            Hold::InFlight {
                until,
                handle: latest,
            } if latest == handle && now < until => Ok((arrival, message)),
            _ => Err(expired()),
        }
    }

    pub fn delete_message(
        &mut self,
        handle: ReceiptHandle,
        now: Timestamp,
    ) -> Result<(), CommandError> {
        let (arrival, _) = self.in_flight(handle, now)?;
        self.remove(arrival);
        Ok(())
    }

    pub fn change_visibility(
        &mut self,
        handle: ReceiptHandle,
        visibility_timeout: Seconds,
        now: Timestamp,
    ) -> Result<(), CommandError> {
        let (_, message) = self.in_flight(handle, now)?;
        message.hold = Hold::InFlight {
            until: now + visibility_timeout,
            handle,
        };
        Ok(())
    }
}
