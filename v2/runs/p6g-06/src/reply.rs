//! Success and failure responses as they appear on the wire.

use serde::Serialize;

use crate::attributes::Attributes;
use crate::error::{Error, ErrorCode};
use crate::ids::{MessageId, ReceiptHandle};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Counts {
    pub visible: usize,
    pub in_flight: usize,
    pub delayed: usize,
}

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
    pub group_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sequence_number: Option<u64>,
}

/// The result members of a successful command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged, rename_all_fields = "PascalCase")]
pub enum Reply {
    Empty {},
    Now {
        now: u64,
    },
    QueueNames {
        queue_names: Vec<String>,
    },
    Attributes {
        attributes: Attributes,
        counts: Counts,
    },
    Sent {
        message_id: MessageId,
        #[serde(skip_serializing_if = "Option::is_none")]
        sequence_number: Option<u64>,
    },
    Messages {
        messages: Vec<ReceivedMessage>,
    },
}

#[derive(Serialize)]
struct Success<'a> {
    ok: bool,
    #[serde(flatten)]
    reply: &'a Reply,
}

#[derive(Serialize)]
struct Failure<'a> {
    ok: bool,
    error: ErrorBody<'a>,
}

#[derive(Serialize)]
struct ErrorBody<'a> {
    code: ErrorCode,
    message: &'a str,
}

/// The single-line JSON text of a response, without the newline.
pub fn render(outcome: &Result<Reply, Error>) -> String {
    let rendered = match outcome {
        Ok(reply) => serde_json::to_string(&Success { ok: true, reply }),
        Err(error) => serde_json::to_string(&Failure {
            ok: false,
            error: ErrorBody {
                code: error.code(),
                message: error.message(),
            },
        }),
    };
    rendered.expect("replies contain only strings, integers and booleans")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn value(outcome: Result<Reply, Error>) -> serde_json::Value {
        serde_json::from_str(&render(&outcome)).unwrap()
    }

    #[test]
    fn empty_success() {
        assert_eq!(value(Ok(Reply::Empty {})), json!({"ok": true}));
    }

    #[test]
    fn result_members_are_pascal_case() {
        assert_eq!(
            value(Ok(Reply::Now { now: 5 })),
            json!({"ok": true, "Now": 5})
        );
        let names = vec!["a".to_owned()];
        assert_eq!(
            value(Ok(Reply::QueueNames { queue_names: names })),
            json!({"ok": true, "QueueNames": ["a"]})
        );
    }

    #[test]
    fn sequence_number_is_omitted_when_absent() {
        let mut counter = 0;
        let reply = Reply::Sent {
            message_id: MessageId::issue(&mut counter),
            sequence_number: None,
        };
        assert_eq!(value(Ok(reply)), json!({"ok": true, "MessageId": "m-1"}));
    }

    #[test]
    fn failure_carries_code_and_message() {
        let error = Error::new(ErrorCode::LimitExceeded, "full");
        assert_eq!(
            value(Err(error)),
            json!({"ok": false, "error": {"code": "LimitExceeded", "message": "full"}})
        );
    }
}
