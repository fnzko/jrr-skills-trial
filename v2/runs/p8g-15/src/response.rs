//! The response lines: what a command returns and how it is written.

use serde::Serialize;

use crate::error::{Error, ErrorCode};
use crate::queue::{Counts, DeliveredMessage, QueueConfig, QueueKind};
use crate::types::{
    DelaySeconds, MaxReceiveCount, MessageId, QueueName, RetentionSeconds, SequenceNumber,
    Timestamp, VisibilityTimeout,
};

/// The queue attributes as `GetQueueAttributes` reports them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct QueueAttributes {
    visibility_timeout: VisibilityTimeout,
    delay_seconds: DelaySeconds,
    retention_seconds: RetentionSeconds,
    fifo: bool,
    content_based_deduplication: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    dead_letter_queue: Option<QueueName>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_receive_count: Option<MaxReceiveCount>,
}

impl From<&QueueConfig> for QueueAttributes {
    fn from(config: &QueueConfig) -> Self {
        let content_based_deduplication = matches!(
            config.kind,
            QueueKind::Fifo {
                content_based_deduplication: true
            }
        );
        Self {
            visibility_timeout: config.visibility_timeout,
            delay_seconds: config.delay,
            retention_seconds: config.retention,
            fifo: config.kind.is_fifo(),
            content_based_deduplication,
            dead_letter_queue: config.dead_letter.as_ref().map(|d| d.queue.clone()),
            max_receive_count: config.dead_letter.as_ref().map(|d| d.max_receive_count),
        }
    }
}

/// The members a successful command adds to `{"ok": true}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum Reply {
    Empty {},
    Attributes {
        #[serde(rename = "Attributes")]
        attributes: QueueAttributes,
        #[serde(rename = "Counts")]
        counts: Counts,
    },
    Queues {
        #[serde(rename = "QueueNames")]
        queue_names: Vec<QueueName>,
    },
    Now {
        #[serde(rename = "Now")]
        now: Timestamp,
    },
    Sent {
        #[serde(rename = "MessageId")]
        message_id: MessageId,
        #[serde(rename = "SequenceNumber", skip_serializing_if = "Option::is_none")]
        sequence_number: Option<SequenceNumber>,
    },
    Received {
        #[serde(rename = "Messages")]
        messages: Vec<DeliveredMessage>,
    },
}

/// One response line.
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum Response {
    Success {
        ok: bool,
        #[serde(flatten)]
        reply: Reply,
    },
    Failure {
        ok: bool,
        error: ErrorBody,
    },
}

#[derive(Debug, Serialize)]
pub struct ErrorBody {
    code: ErrorCode,
    message: String,
}

impl Response {
    pub fn from_result(result: Result<Reply, Error>) -> Self {
        match result {
            Ok(reply) => Self::Success { ok: true, reply },
            Err(error) => Self::Failure {
                ok: false,
                error: ErrorBody {
                    code: error.code(),
                    message: error.message().to_owned(),
                },
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn line(result: Result<Reply, Error>) -> serde_json::Value {
        serde_json::to_value(Response::from_result(result)).unwrap()
    }

    #[test]
    fn empty_success_has_only_ok() {
        assert_eq!(line(Ok(Reply::Empty {})), json!({"ok": true}));
    }

    #[test]
    fn failure_carries_code_and_message() {
        let error = Error::new(ErrorCode::QueueDoesNotExist, "no such queue");
        assert_eq!(
            line(Err(error)),
            json!({"ok": false, "error": {"code": "QueueDoesNotExist", "message": "no such queue"}})
        );
    }

    #[test]
    fn standard_send_omits_the_sequence_number() {
        let mut ids = crate::types::IdAllocator::default();
        let reply = Reply::Sent {
            message_id: ids.next_message(),
            sequence_number: None,
        };
        assert_eq!(line(Ok(reply)), json!({"ok": true, "MessageId": "m-1"}));
    }
}
