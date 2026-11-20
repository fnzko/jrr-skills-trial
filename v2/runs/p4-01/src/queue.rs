//! One queue: its attributes, its messages and its FIFO bookkeeping.

use std::collections::HashMap;

use serde::Serialize;

use crate::error::Error;
use crate::request::SendMessage;
use crate::store::{
    Counts, FifoTag, Message, MessageStore, NotInFlight, ReceiveOptions, ReceivedMessage,
};
use crate::types::{
    Counter, DeadLetterRef, MaxReceiveCount, MessageId, QueueName, ReceiptHandle, Seconds,
    SequenceNumber, Timestamp,
};

pub const DEFAULT_VISIBILITY_TIMEOUT: Seconds = Seconds::new(30);
pub const DEFAULT_DELAY: Seconds = Seconds::new(0);
pub const DEFAULT_RETENTION: Seconds = Seconds::new(345_600);

const MAX_MESSAGES: usize = 100_000;
const DEDUPLICATION_WINDOW: Seconds = Seconds::new(300);

/// The attributes of a queue, as `GetQueueAttributes` reports them and as
/// `CreateQueue` compares them with an existing queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct QueueAttributes<'a> {
    pub visibility_timeout: Seconds,
    pub delay_seconds: Seconds,
    pub retention_seconds: Seconds,
    pub fifo: bool,
    pub content_based_deduplication: bool,
    #[serde(flatten)]
    pub dead_letter: Option<DeadLetterRef<'a>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeadLetter {
    pub queue: QueueName,
    pub max_receive_count: MaxReceiveCount,
}

/// What a successful send reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Sent {
    pub message_id: MessageId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sequence_number: Option<SequenceNumber>,
}

#[derive(Debug)]
pub struct Queue {
    pub visibility_timeout: Seconds,
    pub delay: Seconds,
    pub retention: Seconds,
    pub dead_letter: Option<DeadLetter>,
    kind: Kind,
    messages: MessageStore,
}

#[derive(Debug)]
enum Kind {
    Standard,
    Fifo(FifoState),
}

#[derive(Debug, Default)]
struct FifoState {
    content_based_deduplication: bool,
    sequence: Counter,
    deduplication: DeduplicationWindow,
}

impl Queue {
    /// A new empty queue with `attributes`, except that the dead-letter
    /// configuration is `dead_letter`: the same one with its queue resolved.
    #[must_use]
    pub fn new(attributes: &QueueAttributes<'_>, dead_letter: Option<DeadLetter>) -> Self {
        let kind = if attributes.fifo {
            Kind::Fifo(FifoState {
                content_based_deduplication: attributes.content_based_deduplication,
                ..FifoState::default()
            })
        } else {
            Kind::Standard
        };
        Self {
            visibility_timeout: attributes.visibility_timeout,
            delay: attributes.delay_seconds,
            retention: attributes.retention_seconds,
            dead_letter,
            kind,
            messages: MessageStore::default(),
        }
    }

    #[must_use]
    pub fn attributes(&self) -> QueueAttributes<'_> {
        QueueAttributes {
            visibility_timeout: self.visibility_timeout,
            delay_seconds: self.delay,
            retention_seconds: self.retention,
            fifo: self.is_fifo(),
            content_based_deduplication: match &self.kind {
                Kind::Standard => false,
                Kind::Fifo(fifo) => fifo.content_based_deduplication,
            },
            dead_letter: self.dead_letter.as_ref().map(|dead_letter| DeadLetterRef {
                queue: dead_letter.queue.as_str(),
                max_receive_count: dead_letter.max_receive_count,
            }),
        }
    }

    #[must_use]
    pub fn is_fifo(&self) -> bool {
        matches!(self.kind, Kind::Fifo(_))
    }

    /// Sets `ContentBasedDeduplication`; the caller has rejected `true` for a
    /// standard queue, and `false` there is already the value.
    pub fn set_content_based_deduplication(&mut self, enabled: bool) {
        if let Kind::Fifo(fifo) = &mut self.kind {
            fifo.content_based_deduplication = enabled;
        }
    }

    /// Removes the messages that are past retention at `now`. Every access to
    /// a queue's messages calls this first, which makes retention hold at every
    /// moment as far as any command can observe.
    pub fn expire(&mut self, now: Timestamp) {
        self.messages.expire(now, self.retention);
    }

    #[must_use]
    pub fn counts(&self, now: Timestamp) -> Counts {
        self.messages.counts(now)
    }

    pub fn purge(&mut self) {
        self.messages.clear();
    }

    /// Applies the `SendMessage` rules that depend on the queue, then accepts
    /// the message unless it is a duplicate.
    pub fn send(
        &mut self,
        request: SendMessage,
        now: Timestamp,
        message_ids: &mut Counter,
    ) -> Result<Sent, Error> {
        let SendMessage {
            queue: _,
            body,
            delay,
            group,
            deduplication_id,
        } = request;
        match &mut self.kind {
            Kind::Standard => {
                if group.is_some() || deduplication_id.is_some() {
                    return Err(Error::UnsupportedOperation(
                        "GroupId and DeduplicationId are only for FIFO queues",
                    ));
                }
                ensure_room(&self.messages)?;
                let message_id = message_ids.next_message_id();
                let visible_from = now.plus(delay.unwrap_or(self.delay));
                self.messages
                    .push(Message::new(message_id, body, now, visible_from, None));
                Ok(Sent {
                    message_id,
                    sequence_number: None,
                })
            }
            Kind::Fifo(fifo) => {
                if delay.is_some() {
                    return Err(Error::UnsupportedOperation(
                        "a FIFO queue does not take a per-message DelaySeconds",
                    ));
                }
                let group_id = group.ok_or(Error::MissingParameter("GroupId"))?;
                let key = match &deduplication_id {
                    Some(id) => id.as_str(),
                    None if fifo.content_based_deduplication => body.as_str(),
                    None => return Err(Error::MissingParameter("DeduplicationId")),
                };
                if let Some(original) = fifo.deduplication.find(key, now) {
                    return Ok(original);
                }
                ensure_room(&self.messages)?;
                let message_id = message_ids.next_message_id();
                let sequence_number = fifo.sequence.next_sequence_number();
                let sent = Sent {
                    message_id,
                    sequence_number: Some(sequence_number),
                };
                fifo.deduplication
                    .record(DeduplicationKey(key.to_owned()), now, sent);
                let tag = FifoTag {
                    group_id,
                    sequence_number,
                };
                let visible_from = now.plus(self.delay);
                self.messages
                    .push(Message::new(message_id, body, now, visible_from, Some(tag)));
                Ok(sent)
            }
        }
    }

    /// Delivers up to `max_messages` messages and returns them together with
    /// the messages that must move to the dead-letter queue.
    pub fn receive(
        &mut self,
        now: Timestamp,
        max_messages: usize,
        visibility_timeout: Option<Seconds>,
        receipt_handles: &mut Counter,
    ) -> (Vec<ReceivedMessage>, Vec<Message>) {
        let options = ReceiveOptions {
            max_messages,
            visibility_timeout: visibility_timeout.unwrap_or(self.visibility_timeout),
            dead_letter_after: self.dead_letter.as_ref().map(|d| d.max_receive_count),
        };
        self.messages.receive(now, options, receipt_handles)
    }

    /// Adds a message moved here from a queue whose dead-letter queue this is.
    pub fn accept_dead_letter(&mut self, mut message: Message, now: Timestamp) {
        message.reset_delivery(now);
        match (&mut self.kind, &mut message.fifo) {
            (Kind::Fifo(fifo), Some(tag)) => {
                tag.sequence_number = fifo.sequence.next_sequence_number()
            }
            (Kind::Standard, None) => {}
            (Kind::Fifo(_), None) | (Kind::Standard, Some(_)) => {
                unreachable!("a dead-letter queue always has the type of the queues that use it")
            }
        }
        self.messages.push(message);
    }

    pub fn delete_message(
        &mut self,
        handle: ReceiptHandle,
        now: Timestamp,
    ) -> Result<(), NotInFlight> {
        self.messages.delete(handle, now)
    }

    pub fn change_visibility(
        &mut self,
        handle: ReceiptHandle,
        now: Timestamp,
        timeout: Seconds,
    ) -> Result<(), NotInFlight> {
        self.messages.change_visibility(handle, now, timeout)
    }
}

fn ensure_room(messages: &MessageStore) -> Result<(), Error> {
    if messages.len() < MAX_MESSAGES {
        Ok(())
    } else {
        Err(Error::LimitExceeded(
            "the queue already holds 100000 messages",
        ))
    }
}

/// A deduplication id: an explicit `DeduplicationId` or, failing that, the body.
#[derive(Debug, PartialEq, Eq, Hash)]
struct DeduplicationKey(String);

impl std::borrow::Borrow<str> for DeduplicationKey {
    fn borrow(&self) -> &str {
        &self.0
    }
}

/// The deduplication ids a FIFO queue accepted, with when and as what.
#[derive(Debug, Default)]
struct DeduplicationWindow {
    accepted: HashMap<DeduplicationKey, (Timestamp, Sent)>,
    /// Size of `accepted` at which closed windows are next dropped; doubling it
    /// keeps the cost of dropping them amortized constant per send.
    prune_at: usize,
}

impl DeduplicationWindow {
    const MIN_PRUNE_AT: usize = 1024;

    /// The original send if `key` was accepted less than 300 seconds before `now`.
    fn find(&self, key: &str, now: Timestamp) -> Option<Sent> {
        self.accepted
            .get(key)
            .filter(|(at, _)| is_open(*at, now))
            .map(|&(_, sent)| sent)
    }

    fn record(&mut self, key: DeduplicationKey, now: Timestamp, sent: Sent) {
        self.accepted.insert(key, (now, sent));
        if self.accepted.len() > self.prune_at {
            self.accepted.retain(|_, (at, _)| is_open(*at, now));
            self.prune_at = (self.accepted.len() * 2).max(Self::MIN_PRUNE_AT);
        }
    }
}

fn is_open(accepted_at: Timestamp, now: Timestamp) -> bool {
    now < accepted_at.plus(DEDUPLICATION_WINDOW)
}
