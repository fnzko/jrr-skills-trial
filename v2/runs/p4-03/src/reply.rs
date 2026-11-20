//! Response lines (SPEC section 1). Member names are part of the wire format: result members are
//! PascalCase, and only `ok`, `error`, `code` and `message` are lower case.

use crate::attributes::QueueConfig;
use crate::error::Failure;
use crate::ids::{GroupId, MessageId, QueueName, ReceiptHandle, SequenceNumber};
use crate::queue::{Counts, Message};
use crate::time::{Instant, Seconds};
use serde::Serialize;

/// One response line: a success with its result members, or a failure.
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum Reply<'a> {
    Success {
        ok: bool,
        #[serde(flatten)]
        result: Outcome<'a>,
    },
    Failure {
        ok: bool,
        error: Failure,
    },
}

/// The result members of a successful command. A command with none is `Empty`.
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum Outcome<'a> {
    Empty(Empty),
    Now(NowResult),
    Attributes(AttributesResult<'a>),
    QueueNames(QueueNamesResult<'a>),
    Sent(SentResult),
    Received(ReceiveResult<'a>),
}

#[derive(Debug, Serialize)]
pub struct Empty {}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct NowResult {
    pub now: Instant,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct AttributesResult<'a> {
    attributes: AttributesWire<'a>,
    counts: CountsWire,
}

impl<'a> AttributesResult<'a> {
    pub fn new(config: &'a QueueConfig, counts: Counts) -> AttributesResult<'a> {
        AttributesResult {
            attributes: AttributesWire::from_config(config),
            counts: CountsWire::from(counts),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
struct AttributesWire<'a> {
    visibility_timeout: Seconds,
    delay_seconds: Seconds,
    retention_seconds: Seconds,
    fifo: bool,
    content_based_deduplication: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    dead_letter_queue: Option<&'a QueueName>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_receive_count: Option<u32>,
}

impl<'a> AttributesWire<'a> {
    fn from_config(config: &'a QueueConfig) -> AttributesWire<'a> {
        AttributesWire {
            visibility_timeout: config.visibility_timeout(),
            delay_seconds: config.delay_seconds(),
            retention_seconds: config.retention_seconds(),
            fifo: config.fifo(),
            content_based_deduplication: config.content_based_deduplication(),
            dead_letter_queue: config.dead_letter().map(|dead_letter| dead_letter.queue()),
            max_receive_count: config
                .dead_letter()
                .map(|dead_letter| dead_letter.max_receive_count()),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
struct CountsWire {
    visible: u64,
    in_flight: u64,
    delayed: u64,
}

impl From<Counts> for CountsWire {
    fn from(counts: Counts) -> CountsWire {
        CountsWire {
            visible: counts.visible,
            in_flight: counts.in_flight,
            delayed: counts.delayed,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct QueueNamesResult<'a> {
    pub queue_names: Vec<&'a QueueName>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct SentResult {
    pub message_id: MessageId,
    /// Present only for a FIFO queue.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sequence_number: Option<SequenceNumber>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct ReceiveResult<'a> {
    pub messages: Vec<ReceivedMessage<'a>>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct ReceivedMessage<'a> {
    message_id: MessageId,
    receipt_handle: ReceiptHandle,
    body: &'a str,
    receive_count: u64,
    sent_at: Instant,
    first_received_at: Instant,
    /// Present only for a FIFO queue.
    #[serde(skip_serializing_if = "Option::is_none")]
    group_id: Option<&'a GroupId>,
    /// Present only for a FIFO queue.
    #[serde(skip_serializing_if = "Option::is_none")]
    sequence_number: Option<SequenceNumber>,
}

impl<'a> ReceivedMessage<'a> {
    /// The message as a receive reports it. Only a delivered message is reported, and a
    /// delivery always sets its receipt handle and first receive time.
    pub fn new(message: &'a Message) -> ReceivedMessage<'a> {
        ReceivedMessage {
            message_id: message.id(),
            receipt_handle: message
                .receipt()
                .expect("a delivered message has a receipt handle"),
            body: message.body(),
            receive_count: message.receive_count(),
            sent_at: message.sent_at(),
            first_received_at: message
                .first_received_at()
                .expect("a delivered message has a first receive time"),
            group_id: message.fifo().map(|position| position.group()),
            sequence_number: message.fifo().map(|position| position.sequence()),
        }
    }
}
