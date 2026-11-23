//! The shapes of success and failure responses.

use serde::Serialize;

use crate::attributes::Attributes;
use crate::error::Error;
use crate::ids::QueueName;
use crate::queue::{Counts, Delivery, Sent};
use crate::time::Time;

/// Result of `GetQueueAttributes`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct QueueState<'a> {
    attributes: &'a Attributes,
    counts: Counts,
}

impl<'a> QueueState<'a> {
    pub(crate) fn new(attributes: &'a Attributes, counts: Counts) -> Self {
        Self { attributes, counts }
    }
}

/// Result of `ListQueues`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct QueueNames<'a> {
    queue_names: Vec<&'a QueueName>,
}

impl<'a> QueueNames<'a> {
    pub(crate) fn new(queue_names: Vec<&'a QueueName>) -> Self {
        Self { queue_names }
    }
}

/// Result of `Tick`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Clock {
    now: Time,
}

impl Clock {
    pub(crate) fn new(now: Time) -> Self {
        Self { now }
    }
}

/// Result of `ReceiveMessage`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Messages {
    messages: Vec<Delivery>,
}

impl Messages {
    pub(crate) fn new(messages: Vec<Delivery>) -> Self {
        Self { messages }
    }
}

/// The result members of a successful command.
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum Reply<'a> {
    /// A command with no result members.
    Empty {},
    /// `GetQueueAttributes`.
    QueueState(QueueState<'a>),
    /// `ListQueues`.
    QueueNames(QueueNames<'a>),
    /// `Tick`.
    Clock(Clock),
    /// `SendMessage`.
    Sent(Sent),
    /// `ReceiveMessage`.
    Messages(Messages),
}

/// One response line.
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum Response<'a> {
    /// `{"ok": true, ...}`
    Success {
        /// Always true.
        ok: bool,
        /// The command's result members.
        #[serde(flatten)]
        reply: Reply<'a>,
    },
    /// `{"ok": false, "error": {...}}`
    Failure {
        /// Always false.
        ok: bool,
        /// What went wrong.
        error: Error,
    },
}

impl<'a> Response<'a> {
    pub(crate) fn success(reply: Reply<'a>) -> Self {
        Self::Success { ok: true, reply }
    }

    pub(crate) fn failure(error: Error) -> Self {
        Self::Failure { ok: false, error }
    }
}
