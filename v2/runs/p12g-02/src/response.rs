//! The wire form of command results.

use std::io::{self, Write};

use serde::Serialize;

use crate::error::{ApiError, ErrorCode};
use crate::queue::{Counts, DeadLetterConfig, FifoPlacement};
use crate::types::{Instant, MessageId, QueueName, ReceiptHandle, Seconds, SequenceNumber};

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct AttributesView {
    pub visibility_timeout: Seconds,
    pub delay_seconds: Seconds,
    pub retention_seconds: Seconds,
    pub fifo: bool,
    pub content_based_deduplication: bool,
    #[serde(flatten)]
    pub dead_letter: Option<DeadLetterConfig>,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct ReceivedMessage {
    pub message_id: MessageId,
    pub receipt_handle: ReceiptHandle,
    pub body: String,
    pub receive_count: u32,
    pub sent_at: Instant,
    pub first_received_at: Instant,
    #[serde(flatten)]
    pub fifo: Option<FifoPlacement>,
}

/// The result members of a successful command.
#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(untagged, rename_all = "PascalCase")]
pub enum Output {
    Empty {},
    Attributes {
        #[serde(rename = "Attributes")]
        attributes: AttributesView,
        #[serde(rename = "Counts")]
        counts: Counts,
    },
    QueueNames {
        #[serde(rename = "QueueNames")]
        queue_names: Vec<QueueName>,
    },
    Now {
        #[serde(rename = "Now")]
        now: Instant,
    },
    Sent {
        #[serde(rename = "MessageId")]
        message_id: MessageId,
        #[serde(rename = "SequenceNumber", skip_serializing_if = "Option::is_none")]
        sequence_number: Option<SequenceNumber>,
    },
    Received {
        #[serde(rename = "Messages")]
        messages: Vec<ReceivedMessage>,
    },
}

#[derive(Serialize)]
struct Success<'a> {
    ok: bool,
    #[serde(flatten)]
    output: &'a Output,
}

#[derive(Serialize)]
struct ErrorBody<'a> {
    code: ErrorCode,
    message: &'a str,
}

#[derive(Serialize)]
struct Failure<'a> {
    ok: bool,
    error: ErrorBody<'a>,
}

/// Writes the response line for `result` and flushes it.
pub fn write_response(out: &mut impl Write, result: &Result<Output, ApiError>) -> io::Result<()> {
    match result {
        Ok(output) => serde_json::to_writer(&mut *out, &Success { ok: true, output })?,
        Err(error) => serde_json::to_writer(
            &mut *out,
            &Failure {
                ok: false,
                error: ErrorBody {
                    code: error.code,
                    message: &error.message,
                },
            },
        )?,
    }
    out.write_all(b"\n")?;
    out.flush()
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;

    fn rendered(result: Result<Output, ApiError>) -> Value {
        let mut bytes = Vec::new();
        write_response(&mut bytes, &result).unwrap();
        assert_eq!(bytes.last(), Some(&b'\n'));
        serde_json::from_slice(&bytes).unwrap()
    }

    #[test]
    fn success_without_members() {
        assert_eq!(rendered(Ok(Output::Empty {})), json!({"ok": true}));
    }

    #[test]
    fn success_with_members() {
        let now = Output::Now {
            now: Instant::START.plus(Seconds(7)),
        };
        assert_eq!(rendered(Ok(now)), json!({"ok": true, "Now": 7}));
    }

    #[test]
    fn failure_carries_code_and_message() {
        let error = ApiError::new(ErrorCode::QueueDoesNotExist, "no such queue");
        assert_eq!(
            rendered(Err(error)),
            json!({"ok": false, "error": {"code": "QueueDoesNotExist", "message": "no such queue"}})
        );
    }
}
