//! Queue attributes: the stored form, the partial form a request carries, and their rules.

use serde::{Serialize, Serializer};

use crate::error::{Error, ErrorCode, Result};
use crate::types::{Delay, MaxReceiveCount, QueueName, Retention, VisibilityTimeout};

/// The type of a queue never changes. Content based deduplication only exists on FIFO queues.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Standard,
    Fifo { content_based_deduplication: bool },
}

impl Kind {
    pub fn is_fifo(self) -> bool {
        matches!(self, Self::Fifo { .. })
    }
}

/// Where messages go after too many deliveries. Both members are set or neither is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DeadLetter {
    #[serde(rename = "DeadLetterQueue")]
    pub queue: QueueName,
    #[serde(rename = "MaxReceiveCount")]
    pub max_receive_count: MaxReceiveCount,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attributes {
    pub visibility_timeout: VisibilityTimeout,
    pub delay: Delay,
    pub retention: Retention,
    pub kind: Kind,
    pub dead_letter: Option<DeadLetter>,
}

/// The attributes a request names; everything it leaves out is `None`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AttributeUpdate {
    pub visibility_timeout: Option<VisibilityTimeout>,
    pub delay: Option<Delay>,
    pub retention: Option<Retention>,
    pub fifo: Option<bool>,
    pub content_based_deduplication: Option<bool>,
    pub dead_letter_queue: Option<String>,
    pub max_receive_count: Option<MaxReceiveCount>,
}

impl AttributeUpdate {
    pub fn is_fifo(&self) -> bool {
        self.fifo.unwrap_or(false)
    }

    /// The kind a new queue gets from this request.
    pub fn kind_for_new_queue(&self) -> Result<Kind> {
        let content_based_deduplication = self.content_based_deduplication.unwrap_or(false);
        match (self.is_fifo(), content_based_deduplication) {
            (true, _) => Ok(Kind::Fifo {
                content_based_deduplication,
            }),
            (false, false) => Ok(Kind::Standard),
            (false, true) => Err(content_based_on_standard()),
        }
    }

    /// The requested dead-letter configuration, which must name both members or neither.
    pub fn dead_letter_request(&self) -> Result<Option<(&str, MaxReceiveCount)>> {
        match (&self.dead_letter_queue, self.max_receive_count) {
            (Some(queue), Some(max)) => Ok(Some((queue, max))),
            (None, None) => Ok(None),
            _ => Err(Error::new(
                ErrorCode::InvalidParameterValue,
                "`DeadLetterQueue` and `MaxReceiveCount` must be given together",
            )),
        }
    }
}

pub fn content_based_on_standard() -> Error {
    Error::new(
        ErrorCode::InvalidParameterValue,
        "`ContentBasedDeduplication` needs a FIFO queue",
    )
}

impl Attributes {
    /// Attributes of a new queue: the request's values over the defaults.
    pub fn resolve(update: &AttributeUpdate, kind: Kind, dead_letter: Option<DeadLetter>) -> Self {
        Self {
            visibility_timeout: update.visibility_timeout.unwrap_or_default(),
            delay: update.delay.unwrap_or_default(),
            retention: update.retention.unwrap_or_default(),
            kind,
            dead_letter,
        }
    }

    /// Applies a validated update. A dead-letter configuration can be replaced, never removed.
    pub fn apply(&mut self, update: &AttributeUpdate, dead_letter: Option<DeadLetter>) {
        if let Some(value) = update.visibility_timeout {
            self.visibility_timeout = value;
        }
        if let Some(value) = update.delay {
            self.delay = value;
        }
        if let Some(value) = update.retention {
            self.retention = value;
        }
        if let (
            Kind::Fifo {
                content_based_deduplication,
            },
            Some(value),
        ) = (&mut self.kind, update.content_based_deduplication)
        {
            *content_based_deduplication = value;
        }
        if dead_letter.is_some() {
            self.dead_letter = dead_letter;
        }
    }
}

/// The wire form of the stored attributes.
#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct AttributesView<'a> {
    visibility_timeout: VisibilityTimeout,
    #[serde(rename = "DelaySeconds")]
    delay: Delay,
    #[serde(rename = "RetentionSeconds")]
    retention: Retention,
    fifo: bool,
    content_based_deduplication: bool,
    #[serde(flatten)]
    dead_letter: Option<&'a DeadLetter>,
}

impl Serialize for Attributes {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        AttributesView {
            visibility_timeout: self.visibility_timeout,
            delay: self.delay,
            retention: self.retention,
            fifo: self.kind.is_fifo(),
            content_based_deduplication: matches!(
                self.kind,
                Kind::Fifo {
                    content_based_deduplication: true
                }
            ),
            dead_letter: self.dead_letter.as_ref(),
        }
        .serialize(serializer)
    }
}
