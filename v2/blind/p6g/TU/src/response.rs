//! The lines miniq writes: one success or failure object per command.

use serde::Serialize;

use crate::error::Error;
use crate::queue::{Attributes, Counts, DeadLetter, QueueKind, ReceivedMessage, Sent};
use crate::types::{DelaySeconds, QueueName, RetentionSeconds, Sequence, VisibilityTimeout};

/// The result members of a successful command.
#[derive(Debug, Serialize)]
#[serde(untagged, rename_all_fields = "PascalCase")]
pub(crate) enum Output {
    Nothing {},
    Attributes {
        attributes: AttributesView,
        counts: Counts,
    },
    QueueNames {
        queue_names: Vec<QueueName>,
    },
    Now {
        now: u64,
    },
    Sent {
        message_id: crate::types::MessageId,
        #[serde(skip_serializing_if = "Option::is_none")]
        sequence_number: Option<Sequence>,
    },
    Messages {
        messages: Vec<ReceivedMessage>,
    },
}

impl From<Sent> for Output {
    fn from(sent: Sent) -> Self {
        Self::Sent {
            message_id: sent.message_id,
            sequence_number: sent.sequence,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub(crate) struct AttributesView {
    visibility_timeout: VisibilityTimeout,
    delay_seconds: DelaySeconds,
    retention_seconds: RetentionSeconds,
    fifo: bool,
    content_based_deduplication: bool,
    #[serde(flatten)]
    dead_letter: Option<DeadLetter>,
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
                QueueKind::Fifo {
                    content_based_deduplication: true
                }
            ),
            dead_letter: attributes.dead_letter.clone(),
        }
    }
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

pub(crate) fn render(result: &Result<Output, Error>) -> String {
    match result {
        Ok(output) => serde_json::to_string(&Success { ok: true, output }),
        Err(error) => serde_json::to_string(&Failure { ok: false, error }),
    }
    .expect("response types hold only strings and integers, which always serialize")
}
