//! The JSON objects written to stdout.

use serde::Serialize;

use crate::error::ApiError;
use crate::types::{GroupId, MessageId, QueueName, ReceiptHandle, Seconds, SequenceNumber, Time};

/// One response line.
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum Reply {
    Success(Success),
    Failure(Failure),
}

impl From<Result<Output, ApiError>> for Reply {
    fn from(result: Result<Output, ApiError>) -> Self {
        match result {
            Ok(output) => Reply::Success(Success { ok: true, output }),
            Err(error) => Reply::Failure(Failure { ok: false, error }),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct Success {
    ok: bool,
    #[serde(flatten)]
    output: Output,
}

#[derive(Debug, Serialize)]
pub struct Failure {
    ok: bool,
    error: ApiError,
}

/// The result members of a successful command.
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum Output {
    Empty(Empty),
    Tick(TickOutput),
    QueueNames(QueueNamesOutput),
    Attributes(AttributesOutput),
    Sent(SendOutput),
    Received(ReceiveOutput),
}

#[derive(Debug, Serialize)]
pub struct Empty {}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct TickOutput {
    pub now: Time,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct QueueNamesOutput {
    pub queue_names: Vec<QueueName>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct AttributesOutput {
    pub attributes: AttributesView,
    pub counts: Counts,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct AttributesView {
    pub visibility_timeout: Seconds,
    pub delay_seconds: Seconds,
    pub retention_seconds: Seconds,
    pub fifo: bool,
    pub content_based_deduplication: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dead_letter_queue: Option<QueueName>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_receive_count: Option<u32>,
}

#[derive(Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Counts {
    pub visible: usize,
    pub in_flight: usize,
    pub delayed: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct SendOutput {
    pub message_id: MessageId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sequence_number: Option<SequenceNumber>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct ReceiveOutput {
    pub messages: Vec<ReceivedMessage>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct ReceivedMessage {
    pub message_id: MessageId,
    pub receipt_handle: ReceiptHandle,
    pub body: String,
    pub receive_count: u32,
    pub sent_at: Time,
    pub first_received_at: Time,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_id: Option<GroupId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sequence_number: Option<SequenceNumber>,
}
