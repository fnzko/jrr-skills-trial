//! The result members of successful commands, as they appear on the wire.

use serde::Serialize;

use crate::ids::{MessageId, ReceiptHandle};

#[derive(Debug, Serialize)]
#[serde(untagged, rename_all_fields = "PascalCase")]
pub(crate) enum Outcome {
    Empty {},
    Tick {
        now: u64,
    },
    Sent {
        message_id: MessageId,
        #[serde(skip_serializing_if = "Option::is_none")]
        sequence_number: Option<u64>,
    },
    Received {
        messages: Vec<ReceivedMessage>,
    },
    QueueAttributes {
        attributes: AttributesView,
        counts: Counts,
    },
    QueueNames {
        queue_names: Vec<String>,
    },
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub(crate) struct ReceivedMessage {
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

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub(crate) struct AttributesView {
    pub visibility_timeout: u32,
    pub delay_seconds: u32,
    pub retention_seconds: u32,
    pub fifo: bool,
    pub content_based_deduplication: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dead_letter_queue: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_receive_count: Option<u32>,
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "PascalCase")]
pub(crate) struct Counts {
    pub visible: usize,
    pub in_flight: usize,
    pub delayed: usize,
}
