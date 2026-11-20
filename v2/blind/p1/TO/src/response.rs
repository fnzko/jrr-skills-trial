//! The result members of each command and the success and failure envelopes.

use serde::Serialize;

use crate::error::Error;
use crate::queue::{Counts, Delivered, MessageId};

/// The members a successful command adds to `{"ok": true}`.
#[derive(Debug, Serialize)]
#[serde(untagged, rename_all_fields = "PascalCase")]
pub enum Reply {
    Done {},
    Now {
        now: u64,
    },
    QueueNames {
        queue_names: Vec<String>,
    },
    Attributes {
        attributes: AttributesView,
        counts: Counts,
    },
    Sent {
        message_id: MessageId,
        #[serde(skip_serializing_if = "Option::is_none")]
        sequence_number: Option<u64>,
    },
    Messages {
        messages: Vec<Delivered>,
    },
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct AttributesView {
    pub visibility_timeout: u64,
    pub delay_seconds: u64,
    pub retention_seconds: u64,
    pub fifo: bool,
    pub content_based_deduplication: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dead_letter_queue: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_receive_count: Option<u64>,
}

#[derive(Serialize)]
struct Success {
    ok: bool,
    #[serde(flatten)]
    reply: Reply,
}

#[derive(Serialize)]
struct Failure {
    ok: bool,
    error: Error,
}

/// The response line for a command's outcome, without the trailing newline.
pub fn render(outcome: Result<Reply, Error>) -> String {
    let json = match outcome {
        Ok(reply) => serde_json::to_string(&Success { ok: true, reply }),
        Err(error) => serde_json::to_string(&Failure { ok: false, error }),
    };
    json.expect("responses contain only strings, integers and booleans")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorCode;

    #[test]
    fn success_without_members_is_just_ok() {
        assert_eq!(render(Ok(Reply::Done {})), r#"{"ok":true}"#);
    }

    #[test]
    fn result_members_use_the_protocol_names() {
        assert_eq!(render(Ok(Reply::Now { now: 7 })), r#"{"ok":true,"Now":7}"#);
        assert_eq!(
            render(Ok(Reply::QueueNames {
                queue_names: vec!["a".into(), "b".into()]
            })),
            r#"{"ok":true,"QueueNames":["a","b"]}"#
        );
        assert_eq!(
            render(Ok(Reply::Sent {
                message_id: MessageId::new(3),
                sequence_number: None
            })),
            r#"{"ok":true,"MessageId":"m-3"}"#
        );
        assert_eq!(
            render(Ok(Reply::Sent {
                message_id: MessageId::new(3),
                sequence_number: Some(9)
            })),
            r#"{"ok":true,"MessageId":"m-3","SequenceNumber":9}"#
        );
    }

    #[test]
    fn failures_carry_the_code_and_a_message() {
        let error = Error::new(ErrorCode::LimitExceeded, "full");
        assert_eq!(
            render(Err(error)),
            r#"{"ok":false,"error":{"code":"LimitExceeded","message":"full"}}"#
        );
    }
}
