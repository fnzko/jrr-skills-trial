//! Queue attributes: the stored form, and the partial form a request carries.

use crate::error::{ApiError, ErrorCode};
use crate::request::Arg;
use crate::types::{DelaySeconds, MaxReceiveCount, QueueName, RetentionSeconds, VisibilityTimeout};

/// Whether a queue is standard or FIFO. Content-based deduplication only
/// exists on FIFO queues, so it lives in that variant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Standard,
    Fifo { content_based_deduplication: bool },
}

impl Kind {
    pub fn is_fifo(self) -> bool {
        matches!(self, Self::Fifo { .. })
    }
}

/// `DeadLetterQueue` and `MaxReceiveCount`, which are only ever set together.
#[derive(Clone, Debug)]
pub struct DeadLetter {
    pub queue: QueueName,
    pub max_receive_count: MaxReceiveCount,
}

#[derive(Clone, Debug)]
pub struct Attributes {
    pub visibility_timeout: VisibilityTimeout,
    pub delay_seconds: DelaySeconds,
    pub retention_seconds: RetentionSeconds,
    pub kind: Kind,
    pub dead_letter: Option<DeadLetter>,
}

/// The attributes named in a request, each one optional. Names and values
/// are validated, the combinations between them are not.
#[derive(Debug, Default)]
pub struct AttributeSet {
    pub visibility_timeout: Option<VisibilityTimeout>,
    pub delay_seconds: Option<DelaySeconds>,
    pub retention_seconds: Option<RetentionSeconds>,
    pub fifo: Option<bool>,
    pub content_based_deduplication: Option<bool>,
    dead_letter_queue: Option<String>,
    max_receive_count: Option<MaxReceiveCount>,
}

impl AttributeSet {
    pub fn parse(arg: Arg<'_>) -> Result<Self, ApiError> {
        let mut set = Self::default();
        for (name, value) in arg.object()? {
            if value.is_null() {
                continue;
            }
            let member = Arg::new(name, value);
            match name.as_str() {
                "VisibilityTimeout" => {
                    set.visibility_timeout = Some(member.integer(VisibilityTimeout::new)?);
                }
                "DelaySeconds" => set.delay_seconds = Some(member.integer(DelaySeconds::new)?),
                "RetentionSeconds" => {
                    set.retention_seconds = Some(member.integer(RetentionSeconds::new)?);
                }
                "Fifo" => set.fifo = Some(member.boolean()?),
                "ContentBasedDeduplication" => {
                    set.content_based_deduplication = Some(member.boolean()?);
                }
                "DeadLetterQueue" => set.dead_letter_queue = Some(member.string()?.to_owned()),
                "MaxReceiveCount" => {
                    set.max_receive_count = Some(member.integer(MaxReceiveCount::new)?);
                }
                _ => {
                    return Err(ApiError::new(
                        ErrorCode::InvalidParameterValue,
                        format!("unknown attribute {name}"),
                    ));
                }
            }
        }
        Ok(set)
    }

    /// The requested dead-letter configuration. Naming only one of its two
    /// attributes is an error.
    pub fn dead_letter_request(&self) -> Result<Option<(&str, MaxReceiveCount)>, ApiError> {
        match (&self.dead_letter_queue, self.max_receive_count) {
            (Some(queue), Some(max)) => Ok(Some((queue, max))),
            (None, None) => Ok(None),
            _ => Err(ApiError::new(
                ErrorCode::InvalidParameterValue,
                "DeadLetterQueue and MaxReceiveCount must be given together",
            )),
        }
    }

    /// The kind a queue created from this set gets. The caller has already
    /// rejected `ContentBasedDeduplication` on a standard queue.
    pub fn kind(&self) -> Kind {
        if self.fifo.unwrap_or(false) {
            Kind::Fifo {
                content_based_deduplication: self.content_based_deduplication.unwrap_or(false),
            }
        } else {
            Kind::Standard
        }
    }
}

impl Attributes {
    /// The attributes of a new queue: what the request names, defaults for
    /// the rest.
    pub fn from_request(set: &AttributeSet, dead_letter: Option<DeadLetter>) -> Self {
        Self {
            visibility_timeout: set.visibility_timeout.unwrap_or_default(),
            delay_seconds: set.delay_seconds.unwrap_or_default(),
            retention_seconds: set.retention_seconds.unwrap_or_default(),
            kind: set.kind(),
            dead_letter,
        }
    }

    /// Whether a `CreateQueue` request would produce exactly these
    /// attributes, which makes it an idempotent repeat.
    pub fn matches_request(
        &self,
        set: &AttributeSet,
        dead_letter: Option<(&str, MaxReceiveCount)>,
    ) -> bool {
        let current = self
            .dead_letter
            .as_ref()
            .map(|d| (d.queue.as_str(), d.max_receive_count));
        self.visibility_timeout == set.visibility_timeout.unwrap_or_default()
            && self.delay_seconds == set.delay_seconds.unwrap_or_default()
            && self.retention_seconds == set.retention_seconds.unwrap_or_default()
            && self.kind == set.kind()
            && current == dead_letter
    }

    /// Applies a `SetQueueAttributes` request. Attributes the set leaves out
    /// keep their value, and a dead-letter configuration is replaced but
    /// never removed.
    pub fn update(&mut self, set: &AttributeSet, dead_letter: Option<DeadLetter>) {
        if let Some(value) = set.visibility_timeout {
            self.visibility_timeout = value;
        }
        if let Some(value) = set.delay_seconds {
            self.delay_seconds = value;
        }
        if let Some(value) = set.retention_seconds {
            self.retention_seconds = value;
        }
        if let (
            Some(requested),
            Kind::Fifo {
                content_based_deduplication,
            },
        ) = (set.content_based_deduplication, &mut self.kind)
        {
            *content_based_deduplication = requested;
        }
        if dead_letter.is_some() {
            self.dead_letter = dead_letter;
        }
    }
}
