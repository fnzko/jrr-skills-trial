//! Result members of successful commands.

use serde::Serialize;

use crate::queue::Delivery;
use crate::types::{DeadLetter, MessageId, QueueName, Seconds, SequenceNumber};

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum Reply {
    Empty {},
    Sent(Sent),
    Received(Received),
    Attributes(QueueAttributes),
    QueueNames(QueueNames),
    Clock(Clock),
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Sent {
    pub message_id: MessageId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sequence_number: Option<SequenceNumber>,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Received {
    pub messages: Vec<Delivery>,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct QueueNames {
    pub queue_names: Vec<QueueName>,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Clock {
    pub now: Seconds,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct QueueAttributes {
    pub attributes: AttributeValues,
    pub counts: Counts,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
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

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Counts {
    pub visible: usize,
    pub in_flight: usize,
    pub delayed: usize,
}
