//! The JSON written for each command.

use serde::Serialize;

use crate::error::Error;

/// The result members of a successful command, next to `"ok": true`.
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum Response {
    Empty {},
    Tick(TickResult),
    Attributes(QueueAttributesResult),
    Queues(ListQueuesResult),
    Sent(SendResult),
    Received(ReceiveResult),
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct SendResult {
    pub message_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sequence_number: Option<u64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct ReceiveResult {
    pub messages: Vec<ReceivedMessage>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct ReceivedMessage {
    pub message_id: String,
    pub receipt_handle: String,
    pub body: String,
    pub receive_count: u64,
    pub sent_at: u64,
    pub first_received_at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sequence_number: Option<u64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct QueueAttributesResult {
    pub attributes: AttributesView,
    pub counts: CountsView,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct AttributesView {
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

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct CountsView {
    pub visible: usize,
    pub in_flight: usize,
    pub delayed: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct ListQueuesResult {
    pub queue_names: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct TickResult {
    pub now: u64,
}

#[derive(Serialize)]
struct Success {
    ok: bool,
    #[serde(flatten)]
    result: Response,
}

#[derive(Serialize)]
struct Failure {
    ok: bool,
    error: Error,
}

/// Renders the outcome of one command as one line of JSON without the newline.
pub fn render(outcome: Result<Response, Error>) -> String {
    let rendered = match outcome {
        Ok(result) => serde_json::to_string(&Success { ok: true, result }),
        Err(error) => serde_json::to_string(&Failure { ok: false, error }),
    };
    rendered.expect("responses hold only strings, booleans and integers")
}
