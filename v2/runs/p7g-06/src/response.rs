//! Success and failure responses and their one-line JSON encoding.

use std::io::{self, Write};

use serde::Serialize;

use crate::error::Error;
use crate::queue::{Counts, Settings};
use crate::types::{Body, GroupId, Instant, MessageId, QueueName, ReceiptHandle, Seconds};

/// The attributes of a queue as `GetQueueAttributes` reports them.
#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct AttributesView {
    visibility_timeout: Seconds,
    delay_seconds: Seconds,
    retention_seconds: Seconds,
    fifo: bool,
    content_based_deduplication: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    dead_letter_queue: Option<QueueName>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_receive_count: Option<u32>,
}

impl From<&Settings> for AttributesView {
    fn from(settings: &Settings) -> Self {
        Self {
            visibility_timeout: settings.visibility_timeout,
            delay_seconds: settings.delay,
            retention_seconds: settings.retention,
            fifo: settings.kind.is_fifo(),
            content_based_deduplication: settings.kind.content_based_deduplication(),
            dead_letter_queue: settings.dead_letter.as_ref().map(|d| d.queue.clone()),
            max_receive_count: settings.dead_letter.as_ref().map(|d| d.max_receive_count),
        }
    }
}

/// The identity of an accepted message, as `SendMessage` reports it.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct SentMessage {
    pub message_id: MessageId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sequence_number: Option<u64>,
}

/// A delivered message, as `ReceiveMessage` reports it.
#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct ReceivedMessage {
    pub message_id: MessageId,
    pub receipt_handle: ReceiptHandle,
    pub body: Body,
    pub receive_count: u32,
    pub sent_at: Instant,
    pub first_received_at: Instant,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_id: Option<GroupId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sequence_number: Option<u64>,
}

/// The result members of a successful command.
#[derive(Debug, Serialize)]
#[serde(untagged, rename_all_fields = "PascalCase")]
pub enum Reply {
    Empty {},
    Tick {
        now: Instant,
    },
    Attributes {
        attributes: AttributesView,
        counts: Counts,
    },
    Queues {
        queue_names: Vec<QueueName>,
    },
    Sent(SentMessage),
    Messages {
        messages: Vec<ReceivedMessage>,
    },
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

/// Writes one response line, including the final newline.
pub fn write_response(result: &Result<Reply, Error>, out: &mut impl Write) -> io::Result<()> {
    match result {
        Ok(reply) => serde_json::to_writer(&mut *out, &Success { ok: true, reply })?,
        Err(error) => serde_json::to_writer(&mut *out, &Failure { ok: false, error })?,
    }
    out.write_all(b"\n")
}
