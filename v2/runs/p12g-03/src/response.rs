use serde::Serialize;

use crate::error::Error;
use crate::types::{GroupId, MessageId, QueueName, ReceiptHandle, Seconds, SequenceNumber, Time};

#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum Response {
    Success(Success),
    Failure(Failure),
}

impl Response {
    pub fn success(output: Output) -> Self {
        Response::Success(Success { ok: true, output })
    }

    pub fn failure(error: Error) -> Self {
        Response::Failure(Failure { ok: false, error })
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
    error: Error,
}

/// The result members of a successful command.
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum Output {
    Empty {},
    Now(NowOutput),
    QueueNames(QueueNamesOutput),
    Attributes(AttributesOutput),
    Sent(SentOutput),
    Messages(MessagesOutput),
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct NowOutput {
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
    pub attributes: QueueAttributes,
    pub counts: Counts,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct QueueAttributes {
    pub visibility_timeout: Seconds,
    pub delay_seconds: Seconds,
    pub retention_seconds: Seconds,
    pub fifo: bool,
    pub content_based_deduplication: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dead_letter_queue: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_receive_count: Option<u32>,
}

/// How many messages of a queue are in each state.
#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Counts {
    pub visible: usize,
    pub in_flight: usize,
    pub delayed: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct SentOutput {
    pub message_id: MessageId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sequence_number: Option<SequenceNumber>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct MessagesOutput {
    pub messages: Vec<Received>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Received {
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
