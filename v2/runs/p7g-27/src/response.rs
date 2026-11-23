//! What goes to stdout: one success or failure object per command.

use serde::Serialize;

use crate::attributes::Attributes;
use crate::error::Error;
use crate::queue::{Accepted, Counts, Delivery};
use crate::types::{MessageId, QueueName, Seconds, SequenceNumber, Time};

/// The result members of a successful command.
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum Reply {
    Empty {},
    Sent(Sent),
    Received(ReceivedMessages),
    QueueAttributes(QueueAttributes),
    Queues(QueueNames),
    Clock(Clock),
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Sent {
    message_id: MessageId,
    #[serde(skip_serializing_if = "Option::is_none")]
    sequence_number: Option<SequenceNumber>,
}

impl From<Accepted> for Sent {
    fn from(accepted: Accepted) -> Self {
        Self {
            message_id: accepted.id,
            sequence_number: accepted.sequence,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct ReceivedMessages {
    pub messages: Vec<Delivery>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct QueueNames {
    pub queue_names: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Clock {
    pub now: Time,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct QueueAttributes {
    attributes: AttributesView,
    counts: Counts,
}

impl QueueAttributes {
    pub fn new(attributes: &Attributes, counts: Counts) -> Self {
        Self {
            attributes: attributes.into(),
            counts,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
struct AttributesView {
    visibility_timeout: Seconds,
    #[serde(rename = "DelaySeconds")]
    delay: Seconds,
    #[serde(rename = "RetentionSeconds")]
    retention: Seconds,
    fifo: bool,
    content_based_deduplication: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    dead_letter_queue: Option<QueueName>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_receive_count: Option<u32>,
}

impl From<&Attributes> for AttributesView {
    fn from(attributes: &Attributes) -> Self {
        use crate::attributes::QueueKind;
        let content_based_deduplication = matches!(
            attributes.kind,
            QueueKind::Fifo {
                content_based_deduplication: true
            }
        );
        Self {
            visibility_timeout: attributes.visibility_timeout,
            delay: attributes.delay,
            retention: attributes.retention,
            fifo: attributes.kind.is_fifo(),
            content_based_deduplication,
            dead_letter_queue: attributes.dead_letter.as_ref().map(|d| d.queue().clone()),
            max_receive_count: attributes
                .dead_letter
                .as_ref()
                .map(|d| d.max_receive_count()),
        }
    }
}

#[derive(Serialize)]
struct Success<'a> {
    ok: bool,
    #[serde(flatten)]
    reply: &'a Reply,
}

#[derive(Serialize)]
struct ErrorBody<'a> {
    code: &'a str,
    message: &'a str,
}

#[derive(Serialize)]
struct Failure<'a> {
    ok: bool,
    error: ErrorBody<'a>,
}

/// The output line (without newline) for the outcome of one command.
pub fn render(outcome: &Result<Reply, Error>) -> serde_json::Result<String> {
    match outcome {
        Ok(reply) => serde_json::to_string(&Success { ok: true, reply }),
        Err(error) => serde_json::to_string(&Failure {
            ok: false,
            error: ErrorBody {
                code: error.code().as_str(),
                message: error.message(),
            },
        }),
    }
}

impl From<Vec<Delivery>> for ReceivedMessages {
    fn from(messages: Vec<Delivery>) -> Self {
        Self { messages }
    }
}
