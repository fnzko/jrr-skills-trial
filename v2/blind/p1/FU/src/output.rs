//! The members of a successful response, and the two response envelopes.

use serde::Serialize;

use crate::error::Error;
use crate::ids::{MessageId, SequenceNumber};
use crate::queue::{Counts, Delivered};
use crate::scalar::{
    DelaySeconds, MaxReceiveCount, RetentionSeconds, Timestamp, VisibilityTimeout,
};
use crate::text::QueueName;

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct DeadLetterView<'a> {
    #[serde(rename = "DeadLetterQueue")]
    pub queue: &'a QueueName,
    pub max_receive_count: MaxReceiveCount,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct AttributesView<'a> {
    pub visibility_timeout: VisibilityTimeout,
    pub delay_seconds: DelaySeconds,
    pub retention_seconds: RetentionSeconds,
    pub fifo: bool,
    pub content_based_deduplication: bool,
    #[serde(flatten)]
    pub dead_letter: Option<DeadLetterView<'a>>,
}

#[derive(Debug, Serialize)]
#[serde(untagged, rename_all_fields = "PascalCase")]
pub enum Output<'a> {
    Empty {},
    Now {
        now: Timestamp,
    },
    Attributes {
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
        messages: Vec<Delivered>,
    },
}

#[derive(Serialize)]
pub struct Success<'a> {
    pub ok: bool,
    #[serde(flatten)]
    pub output: &'a Output<'a>,
}

#[derive(Serialize)]
pub struct Failure<'a> {
    pub ok: bool,
    pub error: &'a Error,
}
