use serde::Serialize;

use crate::config::{Config, QueueKind};
use crate::error::Error;
use crate::ids::{MessageId, ReceiptHandle};

/// Successful result members of one command.
#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum Outcome {
    Done {},
    Now {
        #[serde(rename = "Now")]
        now: u64,
    },
    Queues {
        #[serde(rename = "QueueNames")]
        queue_names: Vec<String>,
    },
    Attributes {
        #[serde(rename = "Attributes")]
        attributes: AttributesView,
        #[serde(rename = "Counts")]
        counts: Counts,
    },
    Sent {
        #[serde(rename = "MessageId")]
        message_id: MessageId,
        #[serde(rename = "SequenceNumber", skip_serializing_if = "Option::is_none")]
        sequence_number: Option<u64>,
    },
    Received {
        #[serde(rename = "Messages")]
        messages: Vec<Delivery>,
    },
}

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub struct Counts {
    pub visible: usize,
    pub in_flight: usize,
    pub delayed: usize,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
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

impl AttributesView {
    pub fn of(config: &Config) -> Self {
        let (fifo, content_based_deduplication) = match config.kind {
            QueueKind::Standard => (false, false),
            QueueKind::Fifo {
                content_based_deduplication,
            } => (true, content_based_deduplication),
        };
        Self {
            visibility_timeout: config.visibility_timeout,
            delay_seconds: config.delay_seconds,
            retention_seconds: config.retention_seconds,
            fifo,
            content_based_deduplication,
            dead_letter_queue: config.dead_letter.as_ref().map(|d| d.queue.clone()),
            max_receive_count: config.dead_letter.as_ref().map(|d| d.max_receive_count),
        }
    }
}

/// One message handed out by `ReceiveMessage`.
#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub struct Delivery {
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

#[derive(Serialize)]
#[serde(untagged)]
enum Response<'a> {
    Success {
        ok: bool,
        #[serde(flatten)]
        outcome: &'a Outcome,
    },
    Failure {
        ok: bool,
        error: &'a Error,
    },
}

/// The response line (without the newline) for the result of one command.
pub fn render(result: &Result<Outcome, Error>) -> String {
    let response = match result {
        Ok(outcome) => Response::Success { ok: true, outcome },
        Err(error) => Response::Failure { ok: false, error },
    };
    serde_json::to_string(&response).expect("responses contain only strings, integers and arrays")
}
