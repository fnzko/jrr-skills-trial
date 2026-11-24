//! The success and failure lines written to stdout.

use serde::Serialize;

use crate::attributes::Attributes;
use crate::error::ApiError;
use crate::queue::{Counts, Delivery};
use crate::types::{
    DelaySeconds, MaxReceiveCount, MessageId, RetentionSeconds, SequenceNumber, Time,
    VisibilityTimeout,
};

/// The queue attributes as `GetQueueAttributes` reports them: the first five
/// always, the dead-letter pair only when set.
#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct AttributesView {
    visibility_timeout: VisibilityTimeout,
    delay_seconds: DelaySeconds,
    retention_seconds: RetentionSeconds,
    fifo: bool,
    content_based_deduplication: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    dead_letter_queue: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_receive_count: Option<MaxReceiveCount>,
}

impl From<&Attributes> for AttributesView {
    fn from(attributes: &Attributes) -> Self {
        Self {
            visibility_timeout: attributes.visibility_timeout,
            delay_seconds: attributes.delay_seconds,
            retention_seconds: attributes.retention_seconds,
            fifo: attributes.kind.is_fifo(),
            content_based_deduplication: matches!(
                attributes.kind,
                crate::attributes::Kind::Fifo {
                    content_based_deduplication: true
                }
            ),
            dead_letter_queue: attributes
                .dead_letter
                .as_ref()
                .map(|d| d.queue.as_str().to_owned()),
            max_receive_count: attributes.dead_letter.as_ref().map(|d| d.max_receive_count),
        }
    }
}

/// The result members of a successful command.
#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum Response {
    Empty {},
    #[serde(rename_all = "PascalCase")]
    Tick {
        now: Time,
    },
    #[serde(rename_all = "PascalCase")]
    Queues {
        queue_names: Vec<String>,
    },
    #[serde(rename_all = "PascalCase")]
    QueueAttributes {
        attributes: AttributesView,
        counts: Counts,
    },
    #[serde(rename_all = "PascalCase")]
    Sent {
        message_id: MessageId,
        #[serde(skip_serializing_if = "Option::is_none")]
        sequence_number: Option<SequenceNumber>,
    },
    #[serde(rename_all = "PascalCase")]
    Received {
        messages: Vec<Delivery>,
    },
}

#[derive(Serialize)]
struct Success<'a> {
    ok: bool,
    #[serde(flatten)]
    result: &'a Response,
}

#[derive(Serialize)]
struct Failure<'a> {
    ok: bool,
    error: &'a ApiError,
}

/// The response line, without its trailing newline.
pub fn render(outcome: &Result<Response, ApiError>) -> String {
    let rendered = match outcome {
        Ok(result) => serde_json::to_string(&Success { ok: true, result }),
        Err(error) => serde_json::to_string(&Failure { ok: false, error }),
    };
    rendered.expect("responses hold only strings, integers and booleans, which always serialize")
}
