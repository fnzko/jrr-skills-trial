//! Success results and the JSON envelope of a reply.

use serde::Serialize;

use crate::error::Error;
use crate::ids::{GroupId, MessageId, QueueName, ReceiptHandle, SequenceNumber};

/// Number of messages in each state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Counts {
    pub visible: usize,
    pub in_flight: usize,
    pub delayed: usize,
}

/// The attributes of a queue as `GetQueueAttributes` reports them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct AttributesView {
    pub visibility_timeout: u32,
    pub delay_seconds: u32,
    pub retention_seconds: u32,
    pub fifo: bool,
    pub content_based_deduplication: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dead_letter_queue: Option<QueueName>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_receive_count: Option<u32>,
}

/// One delivered message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct ReceivedMessage {
    pub message_id: MessageId,
    pub receipt_handle: ReceiptHandle,
    pub body: String,
    pub receive_count: u32,
    pub sent_at: u64,
    pub first_received_at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_id: Option<GroupId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sequence_number: Option<SequenceNumber>,
}

/// The result members of a successful command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged, rename_all_fields = "PascalCase")]
pub enum Output {
    Empty {},
    Sent {
        message_id: MessageId,
        #[serde(skip_serializing_if = "Option::is_none")]
        sequence_number: Option<SequenceNumber>,
    },
    Received {
        messages: Vec<ReceivedMessage>,
    },
    QueueAttributes {
        attributes: AttributesView,
        counts: Counts,
    },
    Queues {
        queue_names: Vec<QueueName>,
    },
    Now {
        now: u64,
    },
}

#[derive(Serialize)]
struct Success<'a> {
    ok: bool,
    #[serde(flatten)]
    output: &'a Output,
}

#[derive(Serialize)]
struct Failure<'a> {
    ok: bool,
    error: &'a Error,
}

/// Writes the reply as one JSON object, without a line terminator.
pub fn write_reply(
    writer: impl std::io::Write,
    reply: &Result<Output, Error>,
) -> serde_json::Result<()> {
    match reply {
        Ok(output) => serde_json::to_writer(writer, &Success { ok: true, output }),
        Err(error) => serde_json::to_writer(writer, &Failure { ok: false, error }),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::error::ErrorCode;

    fn render(reply: &Result<Output, Error>) -> serde_json::Value {
        let mut buffer = Vec::new();
        write_reply(&mut buffer, reply).unwrap();
        serde_json::from_slice(&buffer).unwrap()
    }

    #[test]
    fn empty_success_has_only_ok() {
        assert_eq!(render(&Ok(Output::Empty {})), json!({"ok": true}));
    }

    #[test]
    fn sent_omits_sequence_number_on_standard_queues() {
        let output = Output::Sent {
            message_id: MessageId(3),
            sequence_number: None,
        };
        assert_eq!(render(&Ok(output)), json!({"ok": true, "MessageId": "m-3"}));
    }

    #[test]
    fn sent_reports_sequence_number_on_fifo_queues() {
        let output = Output::Sent {
            message_id: MessageId(3),
            sequence_number: Some(SequenceNumber(2)),
        };
        assert_eq!(
            render(&Ok(output)),
            json!({"ok": true, "MessageId": "m-3", "SequenceNumber": 2})
        );
    }

    #[test]
    fn failure_carries_code_and_message() {
        let error = Error::new(ErrorCode::LimitExceeded, "full");
        assert_eq!(
            render(&Err(error)),
            json!({"ok": false, "error": {"code": "LimitExceeded", "message": "full"}})
        );
    }

    #[test]
    fn attributes_leave_out_unset_dead_letter_members() {
        let output = Output::QueueAttributes {
            attributes: AttributesView {
                visibility_timeout: 30,
                delay_seconds: 0,
                retention_seconds: 60,
                fifo: false,
                content_based_deduplication: false,
                dead_letter_queue: None,
                max_receive_count: None,
            },
            counts: Counts {
                visible: 1,
                in_flight: 2,
                delayed: 3,
            },
        };
        assert_eq!(
            render(&Ok(output)),
            json!({
                "ok": true,
                "Attributes": {
                    "VisibilityTimeout": 30, "DelaySeconds": 0, "RetentionSeconds": 60,
                    "Fifo": false, "ContentBasedDeduplication": false
                },
                "Counts": {"Visible": 1, "InFlight": 2, "Delayed": 3}
            })
        );
    }
}
