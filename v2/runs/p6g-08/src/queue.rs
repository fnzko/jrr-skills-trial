//! One queue: its attributes and its messages.
//!
//! Message states depend on the clock, which only moves between commands, so
//! a queue is brought up to date with [`Queue::sync`] before every command
//! that reads or changes its messages. Besides the messages in arrival order
//! the queue keeps indexes that make the common operations logarithmic:
//! `visible` (messages that can be received), `holds` (when delayed and in
//! flight messages change state), `expiry` (when retention removes messages),
//! `receipts` (the live handle of each in-flight message) and `groups` (the
//! messages of each FIFO group, to find the earliest one).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::ops::Bound;

use serde::Serialize;

use crate::error::{Error, ErrorCode};
use crate::request::{AttributeSet, NewMessage};
use crate::types::{
    DelaySeconds, GroupId, Ids, MaxMessages, MaxReceiveCount, MessageId, QueueName, ReceiptHandle,
    RetentionSeconds, Sequence, VisibilityTimeout,
};

pub(crate) const MAX_MESSAGES: usize = 100_000;

/// A send repeating a deduplication id is a duplicate for this many seconds.
const DEDUPLICATION_WINDOW: u64 = 300;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum QueueKind {
    Standard,
    Fifo { content_based_deduplication: bool },
}

impl QueueKind {
    pub(crate) fn is_fifo(self) -> bool {
        matches!(self, Self::Fifo { .. })
    }

    /// The kind a creation request asks for, after the commands have
    /// rejected content-based deduplication on a standard queue.
    pub(crate) fn requested(set: &AttributeSet) -> Self {
        if set.fifo == Some(true) {
            Self::Fifo {
                content_based_deduplication: set.content_based_deduplication.unwrap_or(false),
            }
        } else {
            Self::Standard
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct DeadLetter {
    #[serde(rename = "DeadLetterQueue")]
    pub(crate) queue: QueueName,
    #[serde(rename = "MaxReceiveCount")]
    pub(crate) max_receive_count: MaxReceiveCount,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Attributes {
    pub(crate) visibility_timeout: VisibilityTimeout,
    pub(crate) delay_seconds: DelaySeconds,
    pub(crate) retention_seconds: RetentionSeconds,
    pub(crate) kind: QueueKind,
    pub(crate) dead_letter: Option<DeadLetter>,
}

impl Attributes {
    pub(crate) fn from_request(set: &AttributeSet, dead_letter: Option<DeadLetter>) -> Self {
        Self {
            visibility_timeout: set
                .visibility_timeout
                .unwrap_or(VisibilityTimeout::constant::<30>()),
            delay_seconds: set.delay_seconds.unwrap_or(DelaySeconds::constant::<0>()),
            retention_seconds: set
                .retention_seconds
                .unwrap_or(RetentionSeconds::constant::<345_600>()),
            kind: QueueKind::requested(set),
            dead_letter,
        }
    }

    /// Whether a creation request asks for exactly these attributes. The
    /// dead-letter queue is compared by name, as the request gives it.
    pub(crate) fn matches_request(&self, set: &AttributeSet) -> bool {
        let requested = Self::from_request(set, None);
        let dead_letter = self
            .dead_letter
            .as_ref()
            .map(|d| (d.queue.as_str(), d.max_receive_count));
        let requested_dead_letter = set.dead_letter_queue.as_deref().zip(set.max_receive_count);
        self.visibility_timeout == requested.visibility_timeout
            && self.delay_seconds == requested.delay_seconds
            && self.retention_seconds == requested.retention_seconds
            && self.kind == requested.kind
            && dead_letter == requested_dead_letter
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HoldKind {
    Delayed,
    InFlight(ReceiptHandle),
}

/// Keeps a message invisible while `now < until`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Hold {
    until: u64,
    kind: HoldKind,
}

#[derive(Debug)]
pub(crate) struct Message {
    id: MessageId,
    body: String,
    sent_at: u64,
    receive_count: u64,
    first_received_at: Option<u64>,
    group: Option<GroupId>,
    /// `None` while the message is visible.
    hold: Option<Hold>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Sent {
    pub(crate) message_id: MessageId,
    pub(crate) sequence: Option<Sequence>,
}

#[derive(Debug)]
struct Accepted {
    at: u64,
    sent: Sent,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub(crate) struct ReceivedMessage {
    message_id: MessageId,
    receipt_handle: ReceiptHandle,
    body: String,
    receive_count: u64,
    sent_at: u64,
    first_received_at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    group_id: Option<GroupId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sequence_number: Option<Sequence>,
}

#[derive(Debug, Default)]
pub(crate) struct Received {
    pub(crate) messages: Vec<ReceivedMessage>,
    /// Messages that reached `MaxReceiveCount`, removed from this queue and
    /// still to be handed to the dead-letter queue.
    pub(crate) dead: Vec<Message>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub(crate) struct Counts {
    visible: usize,
    in_flight: usize,
    delayed: usize,
}

#[derive(Debug)]
pub(crate) struct Queue {
    attributes: Attributes,
    messages: BTreeMap<Sequence, Message>,
    next_sequence: Sequence,
    visible: BTreeSet<Sequence>,
    holds: BTreeSet<(u64, Sequence)>,
    expiry: BTreeSet<(u64, Sequence)>,
    receipts: HashMap<ReceiptHandle, Sequence>,
    groups: HashMap<GroupId, BTreeSet<Sequence>>,
    /// Deduplication id (or body) to the latest accepted send. Entries
    /// outlive their messages and are never pruned.
    deduplication: HashMap<String, Accepted>,
}

impl Queue {
    pub(crate) fn new(attributes: Attributes) -> Self {
        Self {
            attributes,
            messages: BTreeMap::new(),
            next_sequence: Sequence::FIRST,
            visible: BTreeSet::new(),
            holds: BTreeSet::new(),
            expiry: BTreeSet::new(),
            receipts: HashMap::new(),
            groups: HashMap::new(),
            deduplication: HashMap::new(),
        }
    }

    pub(crate) fn attributes(&self) -> &Attributes {
        &self.attributes
    }

    /// Applies the passage of time: retention first, then ended delays and
    /// visibility timeouts.
    pub(crate) fn sync(&mut self, now: u64) {
        let retention = self.attributes.retention_seconds.get();
        while let Some(&(sent_at, sequence)) = self.expiry.first()
            && sent_at + retention <= now
        {
            self.remove(sequence);
        }
        while let Some(&(until, sequence)) = self.holds.first()
            && until <= now
        {
            self.holds.pop_first();
            if let Some(Hold {
                kind: HoldKind::InFlight(handle),
                ..
            }) = self.message_mut(sequence).hold.take()
            {
                self.receipts.remove(&handle);
            }
            self.visible.insert(sequence);
        }
    }

    /// Changes the attributes in `set`. Retention is applied before the
    /// change too, so that raising it cannot bring back what the old value
    /// already removed.
    pub(crate) fn set_attributes(
        &mut self,
        now: u64,
        set: &AttributeSet,
        dead_letter: Option<DeadLetter>,
    ) {
        self.sync(now);
        let attributes = &mut self.attributes;
        if let Some(value) = set.visibility_timeout {
            attributes.visibility_timeout = value;
        }
        if let Some(value) = set.delay_seconds {
            attributes.delay_seconds = value;
        }
        if let Some(value) = set.retention_seconds {
            attributes.retention_seconds = value;
        }
        if let (
            QueueKind::Fifo {
                content_based_deduplication,
            },
            Some(value),
        ) = (&mut attributes.kind, set.content_based_deduplication)
        {
            *content_based_deduplication = value;
        }
        if dead_letter.is_some() {
            attributes.dead_letter = dead_letter;
        }
        self.sync(now);
    }

    pub(crate) fn clear_dead_letter(&mut self) {
        self.attributes.dead_letter = None;
    }

    pub(crate) fn counts(&self) -> Counts {
        let delayed = self
            .messages
            .values()
            .filter(|m| {
                matches!(
                    m.hold,
                    Some(Hold {
                        kind: HoldKind::Delayed,
                        ..
                    })
                )
            })
            .count();
        Counts {
            visible: self.visible.len(),
            in_flight: self.messages.len() - self.visible.len() - delayed,
            delayed,
        }
    }

    pub(crate) fn purge(&mut self) {
        self.messages.clear();
        self.visible.clear();
        self.holds.clear();
        self.expiry.clear();
        self.receipts.clear();
        self.groups.clear();
    }

    pub(crate) fn send(&mut self, now: u64, ids: &mut Ids, new: NewMessage) -> Result<Sent, Error> {
        let unsupported = |what: &str| Error::new(ErrorCode::UnsupportedOperation, what);
        let NewMessage {
            body,
            delay_seconds,
            group_id,
            deduplication_id,
        } = new;
        let (group, deduplication_key) = match self.attributes.kind {
            QueueKind::Standard => {
                if group_id.is_some() || deduplication_id.is_some() {
                    return Err(unsupported("GroupId and DeduplicationId need a FIFO queue"));
                }
                (None, None)
            }
            QueueKind::Fifo {
                content_based_deduplication,
            } => {
                if delay_seconds.is_some() {
                    return Err(unsupported(
                        "DelaySeconds per message is not supported on a FIFO queue",
                    ));
                }
                let group = group_id.ok_or_else(|| {
                    Error::new(
                        ErrorCode::MissingParameter,
                        "GroupId is required on a FIFO queue",
                    )
                })?;
                let key = match deduplication_id {
                    Some(id) => id.into_string(),
                    None if content_based_deduplication => body.clone(),
                    None => {
                        return Err(Error::new(
                            ErrorCode::MissingParameter,
                            "DeduplicationId is required unless the queue deduplicates by content",
                        ));
                    }
                };
                if let Some(original) = self.deduplication.get(&key)
                    && now - original.at < DEDUPLICATION_WINDOW
                {
                    return Ok(original.sent);
                }
                (Some(group), Some(key))
            }
        };
        if self.messages.len() >= MAX_MESSAGES {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                format!("the queue already holds {MAX_MESSAGES} messages"),
            ));
        }
        let delay = delay_seconds.unwrap_or(self.attributes.delay_seconds).get();
        let id = ids.next_message_id();
        let sequence = self.insert(Message {
            id,
            body,
            sent_at: now,
            receive_count: 0,
            first_received_at: None,
            group,
            hold: (delay > 0).then_some(Hold {
                until: now + delay,
                kind: HoldKind::Delayed,
            }),
        });
        let sent = Sent {
            message_id: id,
            sequence: self.attributes.kind.is_fifo().then_some(sequence),
        };
        if let Some(key) = deduplication_key {
            self.deduplication.insert(key, Accepted { at: now, sent });
        }
        Ok(sent)
    }

    /// Takes in a message that another queue gave up on. It starts over as
    /// a fresh, visible message; deduplication does not see it.
    pub(crate) fn accept_dead(&mut self, now: u64, mut message: Message) {
        message.receive_count = 0;
        message.first_received_at = None;
        message.hold = None;
        self.insert(message);
        self.sync(now);
    }

    pub(crate) fn receive(
        &mut self,
        now: u64,
        ids: &mut Ids,
        max_messages: MaxMessages,
        visibility_timeout: VisibilityTimeout,
    ) -> Received {
        let max_messages = usize::try_from(max_messages.get())
            .expect("MaxMessages is at most 10, which fits in usize");
        let fifo = self.attributes.kind.is_fifo();
        let mut received = Received::default();
        let mut cursor = Bound::Unbounded;
        while received.messages.len() < max_messages
            && let Some(&sequence) = self.visible.range((cursor, Bound::Unbounded)).next()
        {
            cursor = Bound::Excluded(sequence);
            if !self.is_earliest_of_group(sequence) {
                continue;
            }
            let receive_count = self.message(sequence).receive_count;
            if let Some(dead_letter) = &self.attributes.dead_letter
                && receive_count >= dead_letter.max_receive_count.get()
            {
                received.dead.push(self.remove(sequence));
                continue;
            }
            let until = now + visibility_timeout.get();
            let handle = ids.next_receipt_handle();
            let message = self.message_mut(sequence);
            message.receive_count += 1;
            message.hold = Some(Hold {
                until,
                kind: HoldKind::InFlight(handle),
            });
            let first_received_at = *message.first_received_at.get_or_insert(now);
            received.messages.push(ReceivedMessage {
                message_id: message.id,
                receipt_handle: handle,
                body: message.body.clone(),
                receive_count: message.receive_count,
                sent_at: message.sent_at,
                first_received_at,
                group_id: message.group.clone(),
                sequence_number: fifo.then_some(sequence),
            });
            self.visible.remove(&sequence);
            self.holds.insert((until, sequence));
            self.receipts.insert(handle, sequence);
        }
        received
    }

    /// The message whose latest handle is `handle`, if it is in flight.
    pub(crate) fn in_flight(&self, handle: ReceiptHandle) -> Option<Sequence> {
        self.receipts.get(&handle).copied()
    }

    pub(crate) fn delete(&mut self, sequence: Sequence) {
        self.remove(sequence);
    }

    pub(crate) fn change_visibility(
        &mut self,
        now: u64,
        sequence: Sequence,
        visibility_timeout: VisibilityTimeout,
    ) {
        let until = now + visibility_timeout.get();
        let hold = self
            .message_mut(sequence)
            .hold
            .as_mut()
            .expect("an in-flight message has a hold");
        let previous = std::mem::replace(&mut hold.until, until);
        self.holds.remove(&(previous, sequence));
        self.holds.insert((until, sequence));
        self.sync(now);
    }

    fn message(&self, sequence: Sequence) -> &Message {
        self.messages
            .get(&sequence)
            .expect("the indexes only hold sequences of queued messages")
    }

    fn message_mut(&mut self, sequence: Sequence) -> &mut Message {
        self.messages
            .get_mut(&sequence)
            .expect("the indexes only hold sequences of queued messages")
    }

    /// Adds a message at the end of the arrival order and into the indexes.
    fn insert(&mut self, message: Message) -> Sequence {
        let sequence = self.next_sequence;
        self.next_sequence = sequence.next();
        self.expiry.insert((message.sent_at, sequence));
        match message.hold {
            Some(hold) => {
                self.holds.insert((hold.until, sequence));
            }
            None => {
                self.visible.insert(sequence);
            }
        }
        if let Some(group) = &message.group {
            self.groups
                .entry(group.clone())
                .or_default()
                .insert(sequence);
        }
        self.messages.insert(sequence, message);
        sequence
    }

    fn remove(&mut self, sequence: Sequence) -> Message {
        let message = self
            .messages
            .remove(&sequence)
            .expect("the indexes only hold sequences of queued messages");
        self.expiry.remove(&(message.sent_at, sequence));
        match message.hold {
            None => {
                self.visible.remove(&sequence);
            }
            Some(hold) => {
                self.holds.remove(&(hold.until, sequence));
                if let HoldKind::InFlight(handle) = hold.kind {
                    self.receipts.remove(&handle);
                }
            }
        }
        if let Some(group) = &message.group
            && let Some(members) = self.groups.get_mut(group)
        {
            members.remove(&sequence);
            if members.is_empty() {
                self.groups.remove(group);
            }
        }
        message
    }

    fn is_earliest_of_group(&self, sequence: Sequence) -> bool {
        match &self.message(sequence).group {
            Some(group) => self.groups.get(group).and_then(|m| m.first()) == Some(&sequence),
            None => true,
        }
    }
}
