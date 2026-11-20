//! The wire form of command results.

use serde::Serialize;

use crate::error::Error;
use crate::queue::FifoPosition;
use crate::queue::{Counts, DeadLetter};
use crate::time::Instant;
use crate::values::{
    DelaySeconds, MessageId, QueueName, ReceiptHandle, RetentionSeconds, SequenceNumber,
    VisibilityTimeout,
};

/// The attributes `GetQueueAttributes` reports.
#[derive(Debug, Serialize)]
pub struct AttributesView<'a> {
    #[serde(rename = "VisibilityTimeout")]
    pub visibility_timeout: VisibilityTimeout,
    #[serde(rename = "DelaySeconds")]
    pub delay: DelaySeconds,
    #[serde(rename = "RetentionSeconds")]
    pub retention: RetentionSeconds,
    #[serde(rename = "Fifo")]
    pub fifo: bool,
    #[serde(rename = "ContentBasedDeduplication")]
    pub content_based_deduplication: bool,
    #[serde(flatten)]
    pub dead_letter: Option<&'a DeadLetter>,
}

/// A delivered message.
#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct ReceivedMessage<'a> {
    pub message_id: MessageId,
    pub receipt_handle: ReceiptHandle,
    pub body: &'a str,
    pub receive_count: u64,
    pub sent_at: Instant,
    pub first_received_at: Instant,
    #[serde(flatten)]
    pub fifo: Option<&'a FifoPosition>,
}

/// The members of a successful response besides `ok`.
#[derive(Debug, Serialize)]
#[serde(untagged, rename_all_fields = "PascalCase")]
pub enum Success<'a> {
    Done {},
    Now {
        now: Instant,
    },
    QueueAttributes {
        attributes: AttributesView<'a>,
        counts: Counts,
    },
    QueueNames {
        queue_names: Vec<&'a QueueName>,
    },
    Sent {
        message_id: MessageId,
        #[serde(skip_serializing_if = "Option::is_none")]
        sequence_number: Option<SequenceNumber>,
    },
    Messages {
        messages: Vec<ReceivedMessage<'a>>,
    },
}

#[derive(Serialize)]
struct Succeeded<'a, 'b> {
    ok: bool,
    #[serde(flatten)]
    result: &'b Success<'a>,
}

#[derive(Serialize)]
struct Failed<'a> {
    ok: bool,
    error: &'a Error,
}

/// One response line, without the trailing newline.
pub fn render(outcome: &Result<Success<'_>, Error>) -> Result<String, serde_json::Error> {
    match outcome {
        Ok(result) => serde_json::to_string(&Succeeded { ok: true, result }),
        Err(error) => serde_json::to_string(&Failed { ok: false, error }),
    }
}
