//! Successful results, serialized with the member names of the protocol.

use serde::Serialize;

use crate::ids::{GroupId, MessageId, QueueName, ReceiptHandle, SequenceNumber};
use crate::queue::DeadLetter;
use crate::time::{Seconds, Timestamp};

/// The result members of a successful command, flattened next to `"ok": true`.
#[derive(Debug, PartialEq, Serialize)]
#[serde(untagged)]
pub enum Reply {
    Done {},
    Attributes(QueueDescription),
    Queues(QueueList),
    Time(Time),
    Sent(Sent),
    Received(Received),
}

#[derive(Debug, PartialEq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct QueueDescription {
    pub attributes: AttributeValues,
    pub counts: Counts,
}

#[derive(Debug, PartialEq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct AttributeValues {
    pub visibility_timeout: Seconds,
    pub delay_seconds: Seconds,
    pub retention_seconds: Seconds,
    pub fifo: bool,
    pub content_based_deduplication: bool,
    #[serde(flatten)]
    pub dead_letter: Option<DeadLetter>,
}

#[derive(Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Counts {
    pub visible: usize,
    pub in_flight: usize,
    pub delayed: usize,
}

#[derive(Debug, PartialEq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct QueueList {
    pub queue_names: Vec<QueueName>,
}

#[derive(Debug, PartialEq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Time {
    pub now: Timestamp,
}

/// Also the memory of a FIFO send, so a duplicate can answer with the original result.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Sent {
    pub message_id: MessageId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sequence_number: Option<SequenceNumber>,
}

#[derive(Debug, PartialEq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Received {
    pub messages: Vec<ReceivedMessage>,
}

#[derive(Debug, PartialEq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct ReceivedMessage {
    pub message_id: MessageId,
    pub receipt_handle: ReceiptHandle,
    pub body: String,
    pub receive_count: u32,
    pub sent_at: Timestamp,
    pub first_received_at: Timestamp,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_id: Option<GroupId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sequence_number: Option<SequenceNumber>,
}
