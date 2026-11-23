use serde::Serialize;

use crate::attributes::{Attributes, Kind};
use crate::error::Error;
use crate::queue::{Counts, Delivery, SequenceNumber};
use crate::types::{MessageId, QueueName, Time};

/// The result members of a successful command.
#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(untagged, rename_all_fields = "PascalCase")]
pub enum Reply {
    Empty {},
    Ticked {
        now: Time,
    },
    Queues {
        queue_names: Vec<QueueName>,
    },
    Attributes {
        attributes: AttributesView,
        counts: Counts,
    },
    Sent {
        message_id: MessageId,
        #[serde(skip_serializing_if = "Option::is_none")]
        sequence_number: Option<SequenceNumber>,
    },
    Received {
        messages: Vec<Delivery>,
    },
}

/// A queue's attributes as `GetQueueAttributes` reports them.
#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub struct AttributesView {
    visibility_timeout: u32,
    delay_seconds: u32,
    retention_seconds: u32,
    fifo: bool,
    content_based_deduplication: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    dead_letter_queue: Option<QueueName>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_receive_count: Option<u32>,
}

impl From<&Attributes> for AttributesView {
    fn from(attributes: &Attributes) -> Self {
        let content_based_deduplication = matches!(
            attributes.kind,
            Kind::Fifo {
                content_based_deduplication: true
            }
        );
        Self {
            visibility_timeout: attributes.visibility_timeout.get(),
            delay_seconds: attributes.delay.get(),
            retention_seconds: attributes.retention.get(),
            fifo: attributes.kind.is_fifo(),
            content_based_deduplication,
            dead_letter_queue: attributes.dead_letter.as_ref().map(|d| d.queue.clone()),
            max_receive_count: attributes
                .dead_letter
                .as_ref()
                .map(|d| d.max_receive_count.get()),
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
struct Failure<'a> {
    ok: bool,
    error: &'a Error,
}

/// The single-line JSON response for a command's outcome, without the newline.
pub fn render(outcome: &Result<Reply, Error>) -> String {
    let rendered = match outcome {
        Ok(reply) => serde_json::to_string(&Success { ok: true, reply }),
        Err(error) => serde_json::to_string(&Failure { ok: false, error }),
    };
    rendered.expect("replies are structs with string keys and always serialize")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorCode;

    #[test]
    fn renders_success_members_next_to_ok() {
        let reply = Ok(Reply::Ticked { now: Time::START });
        assert_eq!(render(&reply), r#"{"ok":true,"Now":0}"#);
        assert_eq!(render(&Ok(Reply::Empty {})), r#"{"ok":true}"#);
    }

    #[test]
    fn renders_failures_with_code_and_message() {
        let failure = Err(Error::new(ErrorCode::LimitExceeded, "full"));
        assert_eq!(
            render(&failure),
            r#"{"ok":false,"error":{"code":"LimitExceeded","message":"full"}}"#
        );
    }
}
