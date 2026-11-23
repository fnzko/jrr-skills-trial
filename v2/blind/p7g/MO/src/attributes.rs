//! Queue attributes: the stored form, the requested form and the wire view.

use serde::Serialize;

use crate::types::{DelaySeconds, MaxReceiveCount, QueueName, RetentionSeconds, VisibilityTimeout};

/// The queue type. Content-based deduplication exists only on FIFO queues,
/// so a standard queue cannot carry it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueKind {
    Standard,
    Fifo { content_based_deduplication: bool },
}

impl QueueKind {
    pub fn is_fifo(self) -> bool {
        matches!(self, QueueKind::Fifo { .. })
    }
}

/// Where messages go after too many deliveries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeadLetter {
    pub queue: QueueName,
    pub max_receive_count: MaxReceiveCount,
}

/// Every attribute of an existing queue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueAttributes {
    pub visibility_timeout: VisibilityTimeout,
    pub delay: DelaySeconds,
    pub retention: RetentionSeconds,
    pub kind: QueueKind,
    pub dead_letter: Option<DeadLetter>,
}

/// A validated `SetQueueAttributes` change: the attributes that are present.
#[derive(Debug, Default)]
pub struct Update {
    pub visibility_timeout: Option<VisibilityTimeout>,
    pub delay: Option<DelaySeconds>,
    pub retention: Option<RetentionSeconds>,
    pub content_based_deduplication: Option<bool>,
    pub dead_letter: Option<DeadLetter>,
}

impl QueueAttributes {
    pub fn apply(&mut self, update: Update) {
        self.visibility_timeout = update.visibility_timeout.unwrap_or(self.visibility_timeout);
        self.delay = update.delay.unwrap_or(self.delay);
        self.retention = update.retention.unwrap_or(self.retention);
        if let (
            QueueKind::Fifo {
                content_based_deduplication,
            },
            Some(requested),
        ) = (&mut self.kind, update.content_based_deduplication)
        {
            *content_based_deduplication = requested;
        }
        if update.dead_letter.is_some() {
            self.dead_letter = update.dead_letter;
        }
    }

    pub fn view(&self) -> AttributesView<'_> {
        let (fifo, content_based_deduplication) = match self.kind {
            QueueKind::Standard => (false, false),
            QueueKind::Fifo {
                content_based_deduplication,
            } => (true, content_based_deduplication),
        };
        AttributesView {
            visibility_timeout: self.visibility_timeout,
            delay_seconds: self.delay,
            retention_seconds: self.retention,
            fifo,
            content_based_deduplication,
            dead_letter_queue: self.dead_letter.as_ref().map(|d| &d.queue),
            max_receive_count: self.dead_letter.as_ref().map(|d| d.max_receive_count),
        }
    }
}

/// The `Attributes` object of `GetQueueAttributes`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct AttributesView<'a> {
    visibility_timeout: VisibilityTimeout,
    delay_seconds: DelaySeconds,
    retention_seconds: RetentionSeconds,
    fifo: bool,
    content_based_deduplication: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    dead_letter_queue: Option<&'a QueueName>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_receive_count: Option<MaxReceiveCount>,
}

/// The dead-letter members of a request, which must come as a pair.
#[derive(Debug, Default, PartialEq, Eq)]
pub enum DeadLetterRequest {
    #[default]
    Absent,
    QueueOnly,
    CountOnly,
    Both {
        queue: String,
        max_receive_count: MaxReceiveCount,
    },
}

/// The attributes of a request as given, each validated on its own but not
/// yet checked against the others or against existing queues.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct AttributeRequest {
    pub visibility_timeout: Option<VisibilityTimeout>,
    pub delay: Option<DelaySeconds>,
    pub retention: Option<RetentionSeconds>,
    pub fifo: Option<bool>,
    pub content_based_deduplication: Option<bool>,
    pub dead_letter: DeadLetterRequest,
}
