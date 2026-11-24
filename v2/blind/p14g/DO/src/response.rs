//! The response lines.

use serde::Serialize;

use crate::error::{Code, Error};
use crate::queue::Sent;
use crate::store::{Counts, Delivery};
use crate::types::{
    Body, GroupId, MessageId, QueueName, ReceiptHandle, Seconds, SequenceNumber, Time,
};

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct QueueAttributes {
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

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct ReceivedMessage {
    pub message_id: MessageId,
    pub receipt_handle: ReceiptHandle,
    pub body: Body,
    pub receive_count: u32,
    pub sent_at: Time,
    pub first_received_at: Time,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_id: Option<GroupId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sequence_number: Option<SequenceNumber>,
}

impl From<Delivery> for ReceivedMessage {
    fn from(delivery: Delivery) -> Self {
        let (group_id, sequence_number) = match delivery.fifo {
            Some(info) => (Some(info.group), Some(info.sequence)),
            None => (None, None),
        };
        Self {
            message_id: delivery.id,
            receipt_handle: delivery.handle,
            body: delivery.body,
            receive_count: delivery.receive_count,
            sent_at: delivery.sent_at,
            first_received_at: delivery.first_received_at,
            group_id,
            sequence_number,
        }
    }
}

/// The result members of a successful command.
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum Reply {
    Empty {},
    Attributes(AttributesReply),
    QueueNames(QueueNamesReply),
    Now(NowReply),
    Sent(SentReply),
    Messages(MessagesReply),
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct AttributesReply {
    pub attributes: QueueAttributes,
    pub counts: Counts,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct QueueNamesReply {
    pub queue_names: Vec<QueueName>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct NowReply {
    pub now: Time,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct SentReply {
    pub message_id: MessageId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sequence_number: Option<SequenceNumber>,
}

impl From<Sent> for SentReply {
    fn from(sent: Sent) -> Self {
        Self {
            message_id: sent.id,
            sequence_number: sent.sequence,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct MessagesReply {
    pub messages: Vec<ReceivedMessage>,
}

#[derive(Serialize)]
struct Success<'a> {
    ok: bool,
    #[serde(flatten)]
    reply: &'a Reply,
}

#[derive(Serialize)]
struct Failure {
    ok: bool,
    error: ErrorBody,
}

#[derive(Serialize)]
struct ErrorBody {
    code: Code,
    message: String,
}

/// Renders the outcome of a command as one response line, without the trailing newline.
pub fn render(outcome: &Result<Reply, Error>) -> serde_json::Result<String> {
    match outcome {
        Ok(reply) => serde_json::to_string(&Success { ok: true, reply }),
        Err(error) => serde_json::to_string(&Failure {
            ok: false,
            error: ErrorBody {
                code: error.code(),
                message: error.to_string(),
            },
        }),
    }
}
