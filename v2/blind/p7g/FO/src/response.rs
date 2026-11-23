//! The JSON objects written to stdout.

use std::io::{self, Write};

use serde::Serialize;

use crate::error::Error;
use crate::queue::{Config, Counts, Delivery, Sent};
use crate::types::{QueueName, Time};

#[derive(Debug, Serialize)]
pub struct Nothing {}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct QueueAttributes {
    attributes: AttributesView,
    counts: Counts,
}

impl QueueAttributes {
    pub fn new(config: &Config, counts: Counts) -> Self {
        Self {
            attributes: AttributesView::from(config),
            counts,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
struct AttributesView {
    visibility_timeout: u32,
    delay_seconds: u32,
    retention_seconds: u32,
    fifo: bool,
    content_based_deduplication: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    dead_letter_queue: Option<QueueName>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_receive_count: Option<u32>,
}

impl From<&Config> for AttributesView {
    fn from(config: &Config) -> Self {
        use crate::queue::Kind;
        Self {
            visibility_timeout: config.visibility_timeout.get(),
            delay_seconds: config.delay.get(),
            retention_seconds: config.retention.get(),
            fifo: config.kind.is_fifo(),
            content_based_deduplication: matches!(
                config.kind,
                Kind::Fifo {
                    content_based_deduplication: true
                }
            ),
            dead_letter_queue: config.dead_letter.as_ref().map(|d| d.queue.clone()),
            max_receive_count: config
                .dead_letter
                .as_ref()
                .map(|d| d.max_receive_count.get()),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct QueueNames {
    pub queue_names: Vec<QueueName>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Now {
    pub now: Time,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Messages {
    pub messages: Vec<Delivery>,
}

/// The result members of a successful command.
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum Reply {
    Nothing(Nothing),
    QueueAttributes(QueueAttributes),
    QueueNames(QueueNames),
    Now(Now),
    Sent(Sent),
    Messages(Messages),
}

impl Reply {
    pub fn nothing() -> Self {
        Reply::Nothing(Nothing {})
    }
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

/// Writes one response line and flushes it.
pub fn write_response(out: &mut impl Write, result: &Result<Reply, Error>) -> io::Result<()> {
    match result {
        Ok(reply) => serde_json::to_writer(&mut *out, &Success { ok: true, reply })?,
        Err(error) => serde_json::to_writer(&mut *out, &Failure { ok: false, error })?,
    }
    out.write_all(b"\n")?;
    out.flush()
}
