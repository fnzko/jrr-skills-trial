//! Success payloads and the response envelopes.

use serde::Serialize;

use crate::attributes::{Attributes, QueueKind};
use crate::error::Error;
use crate::names::{GroupId, MessageId, ReceiptHandle};
use crate::queue::Counts;

/// A success payload; the members listed for the command, flattened into the
/// response object.
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum Reply {
    Done(Done),
    Description(Description),
    Queues(QueueList),
    Clock(Clock),
    Sent(Sent),
    Received(Received),
}

/// Result of commands that return nothing.
#[derive(Debug, Serialize)]
pub struct Done {}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Description {
    pub attributes: AttributesView,
    pub counts: Counts,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct AttributesView {
    visibility_timeout: u64,
    delay_seconds: u64,
    retention_seconds: u64,
    fifo: bool,
    content_based_deduplication: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    dead_letter_queue: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_receive_count: Option<u64>,
}

impl From<&Attributes> for AttributesView {
    fn from(attributes: &Attributes) -> Self {
        let (fifo, content_based_deduplication) = match attributes.kind {
            QueueKind::Standard => (false, false),
            QueueKind::Fifo {
                content_based_deduplication,
            } => (true, content_based_deduplication),
        };
        Self {
            visibility_timeout: attributes.visibility_timeout,
            delay_seconds: attributes.delay,
            retention_seconds: attributes.retention,
            fifo,
            content_based_deduplication,
            dead_letter_queue: attributes.dead_letter.as_ref().map(|d| d.queue.clone()),
            max_receive_count: attributes.dead_letter.as_ref().map(|d| d.max_receive_count),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct QueueList {
    pub queue_names: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Clock {
    pub now: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Sent {
    pub message_id: MessageId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sequence_number: Option<u64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Received {
    pub messages: Vec<ReceivedMessage>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct ReceivedMessage {
    pub message_id: MessageId,
    pub receipt_handle: ReceiptHandle,
    pub body: String,
    pub receive_count: u64,
    pub sent_at: u64,
    pub first_received_at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_id: Option<GroupId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sequence_number: Option<u64>,
}

#[derive(Serialize)]
struct Success<'a> {
    ok: bool,
    #[serde(flatten)]
    reply: &'a Reply,
}

#[derive(Serialize)]
struct Failure<'a> {
    ok: bool,
    error: &'a Error,
}

/// The single response line, without its line ending.
pub fn render(outcome: &Result<Reply, Error>) -> Result<String, serde_json::Error> {
    match outcome {
        Ok(reply) => serde_json::to_string(&Success { ok: true, reply }),
        Err(error) => serde_json::to_string(&Failure { ok: false, error }),
    }
}
