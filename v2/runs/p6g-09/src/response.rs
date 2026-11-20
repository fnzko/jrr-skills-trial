//! The result members of each command and the success/failure envelope.

use serde::Serialize;

use crate::attributes::Attributes;
use crate::error::Error;
use crate::queue::{Counts, Delivery};
use crate::types::{MessageId, QueueName, SequenceNumber, Time};

/// Attributes as `GetQueueAttributes` reports them.
#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct AttributesView {
    visibility_timeout: u64,
    delay_seconds: u64,
    retention_seconds: u64,
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
            crate::attributes::QueueKind::Fifo {
                content_based_deduplication: true
            }
        );
        Self {
            visibility_timeout: attributes.visibility_timeout.0,
            delay_seconds: attributes.delay.0,
            retention_seconds: attributes.retention.0,
            fifo: attributes.kind.is_fifo(),
            content_based_deduplication,
            dead_letter_queue: attributes.dead_letter.as_ref().map(|d| d.queue.clone()),
            max_receive_count: attributes.dead_letter.as_ref().map(|d| d.max_receive_count),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(untagged, rename_all_fields = "PascalCase")]
pub enum Output {
    Empty {},
    Now {
        now: Time,
    },
    Sent {
        message_id: MessageId,
        #[serde(skip_serializing_if = "Option::is_none")]
        sequence_number: Option<SequenceNumber>,
    },
    Messages {
        messages: Vec<Delivery>,
    },
    Attributes {
        attributes: AttributesView,
        counts: Counts,
    },
    QueueNames {
        queue_names: Vec<QueueName>,
    },
}

#[derive(Serialize)]
struct Success {
    ok: bool,
    #[serde(flatten)]
    output: Output,
}

#[derive(Serialize)]
struct Failure {
    ok: bool,
    error: Error,
}

/// Renders one response line, without the trailing newline.
pub fn render(result: Result<Output, Error>) -> String {
    let rendered = match result {
        Ok(output) => serde_json::to_string(&Success { ok: true, output }),
        Err(error) => serde_json::to_string(&Failure { ok: false, error }),
    };
    rendered.expect("responses contain only strings, numbers and string-keyed objects")
}
