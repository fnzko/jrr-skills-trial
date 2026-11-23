//! The JSON shapes of success and failure responses.

use serde::Serialize;

use crate::attributes::{QueueAttributes, QueueKind};
use crate::error::Error;
use crate::queue::{Accepted, Counts, Delivered};
use crate::types::{MessageId, QueueName, Seconds, SequenceNumber, Time};

/// The attributes of a queue as reported by `GetQueueAttributes`; unset ones are left out.
#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct AttributesView {
    visibility_timeout: Seconds,
    delay_seconds: Seconds,
    retention_seconds: Seconds,
    fifo: bool,
    content_based_deduplication: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    dead_letter_queue: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_receive_count: Option<u32>,
}

impl From<&QueueAttributes> for AttributesView {
    fn from(attributes: &QueueAttributes) -> Self {
        let (fifo, content_based_deduplication) = match attributes.kind {
            QueueKind::Standard => (false, false),
            QueueKind::Fifo {
                content_based_deduplication,
            } => (true, content_based_deduplication),
        };
        Self {
            visibility_timeout: attributes.visibility_timeout,
            delay_seconds: attributes.delay,
            retention_seconds: attributes.retention,
            fifo,
            content_based_deduplication,
            dead_letter_queue: attributes.dead_letter.as_ref().map(|d| d.queue.clone()),
            max_receive_count: attributes.dead_letter.as_ref().map(|d| d.max_receive_count),
        }
    }
}

/// The result members of a successful command.
#[derive(Debug, Serialize)]
#[serde(untagged, rename_all_fields = "PascalCase")]
pub enum Reply {
    Done {},
    Attributes {
        attributes: AttributesView,
        counts: Counts,
    },
    QueueNames {
        queue_names: Vec<QueueName>,
    },
    Now {
        now: Time,
    },
    Sent {
        message_id: MessageId,
        #[serde(skip_serializing_if = "Option::is_none")]
        sequence_number: Option<SequenceNumber>,
    },
    Messages {
        messages: Vec<Delivered>,
    },
}

impl From<Accepted> for Reply {
    fn from(accepted: Accepted) -> Self {
        Reply::Sent {
            message_id: accepted.message_id,
            sequence_number: accepted.sequence,
        }
    }
}

/// One response line, ready to serialize.
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum Response {
    Success(Success),
    Failure(Failure),
}

#[derive(Debug, Serialize)]
pub struct Success {
    ok: bool,
    #[serde(flatten)]
    reply: Reply,
}

#[derive(Debug, Serialize)]
pub struct Failure {
    ok: bool,
    error: Error,
}

impl From<Result<Reply, Error>> for Response {
    fn from(result: Result<Reply, Error>) -> Self {
        match result {
            Ok(reply) => Response::Success(Success { ok: true, reply }),
            Err(error) => Response::Failure(Failure { ok: false, error }),
        }
    }
}
