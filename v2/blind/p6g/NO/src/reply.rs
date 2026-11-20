//! The result members of successful commands, as they appear on the wire.

use std::time::Duration;

use serde::{Serialize, Serializer};

use crate::queue::DeadLetter;
use crate::types::{GroupId, MessageId, QueueName, ReceiptHandle};

fn seconds<S: Serializer>(value: &Duration, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_u64(value.as_secs())
}

/// The sequence number of a message in a FIFO queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct SequenceNumber(pub(crate) u64);

#[derive(Debug, Serialize)]
pub(crate) struct Counts {
    #[serde(rename = "Visible")]
    pub(crate) visible: usize,
    #[serde(rename = "InFlight")]
    pub(crate) in_flight: usize,
    #[serde(rename = "Delayed")]
    pub(crate) delayed: usize,
}

#[derive(Debug, Serialize)]
pub(crate) struct Attributes {
    #[serde(rename = "VisibilityTimeout", serialize_with = "seconds")]
    pub(crate) visibility_timeout: Duration,
    #[serde(rename = "DelaySeconds", serialize_with = "seconds")]
    pub(crate) delay: Duration,
    #[serde(rename = "RetentionSeconds", serialize_with = "seconds")]
    pub(crate) retention: Duration,
    #[serde(rename = "Fifo")]
    pub(crate) fifo: bool,
    #[serde(rename = "ContentBasedDeduplication")]
    pub(crate) content_based_deduplication: bool,
    #[serde(flatten)]
    pub(crate) dead_letter: Option<DeadLetter>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ReceivedMessage {
    #[serde(rename = "MessageId")]
    pub(crate) message_id: MessageId,
    #[serde(rename = "ReceiptHandle")]
    pub(crate) receipt_handle: ReceiptHandle,
    #[serde(rename = "Body")]
    pub(crate) body: String,
    #[serde(rename = "ReceiveCount")]
    pub(crate) receive_count: u32,
    #[serde(rename = "SentAt", serialize_with = "seconds")]
    pub(crate) sent_at: Duration,
    #[serde(rename = "FirstReceivedAt", serialize_with = "seconds")]
    pub(crate) first_received_at: Duration,
    #[serde(rename = "GroupId", skip_serializing_if = "Option::is_none")]
    pub(crate) group_id: Option<GroupId>,
    #[serde(rename = "SequenceNumber", skip_serializing_if = "Option::is_none")]
    pub(crate) sequence_number: Option<SequenceNumber>,
}

/// What a successful command returns, next to `"ok": true`.
#[derive(Debug, Serialize)]
#[serde(untagged, rename_all_fields = "PascalCase")]
pub(crate) enum Reply {
    Done {},
    Clock {
        #[serde(serialize_with = "seconds")]
        now: Duration,
    },
    Queues {
        queue_names: Vec<QueueName>,
    },
    QueueAttributes {
        attributes: Attributes,
        counts: Counts,
    },
    Sent {
        message_id: MessageId,
        #[serde(skip_serializing_if = "Option::is_none")]
        sequence_number: Option<SequenceNumber>,
    },
    Received {
        messages: Vec<ReceivedMessage>,
    },
}
