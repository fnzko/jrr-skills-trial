//! Queue attributes: their types, defaults, validation and JSON form.

use std::ops::RangeInclusive;

use serde_json::{Value, json};

use crate::error::{Error, ErrorCode, Result};
use crate::request::Params;
use crate::time::Seconds;

const ATTRIBUTE_NAMES: [&str; 7] = [
    "VisibilityTimeout",
    "DelaySeconds",
    "RetentionSeconds",
    "Fifo",
    "ContentBasedDeduplication",
    "DeadLetterQueue",
    "MaxReceiveCount",
];

/// Allowed seconds for a visibility timeout, queue attribute or per delivery.
pub const VISIBILITY_TIMEOUT_RANGE: RangeInclusive<u64> = 0..=43_200;
/// Allowed seconds for a delivery delay, queue attribute or per message.
pub const DELAY_RANGE: RangeInclusive<u64> = 0..=900;
const RETENTION_RANGE: RangeInclusive<u64> = 60..=1_209_600;
const MAX_RECEIVE_COUNT_RANGE: RangeInclusive<u64> = 1..=1000;

/// Where messages go after too many deliveries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeadLetter {
    pub queue: String,
    pub max_receive_count: u64,
}

/// The type of a queue. Content-based deduplication only exists on FIFO
/// queues, so a standard queue cannot have it switched on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QueueKind {
    Standard,
    Fifo { content_based_deduplication: bool },
}

impl QueueKind {
    pub fn is_fifo(self) -> bool {
        matches!(self, Self::Fifo { .. })
    }

    pub fn content_based_deduplication(self) -> bool {
        matches!(
            self,
            Self::Fifo {
                content_based_deduplication: true
            }
        )
    }
}

/// The complete attribute set of a queue.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attributes {
    pub visibility_timeout: Seconds,
    pub delay: Seconds,
    pub retention: Seconds,
    pub kind: QueueKind,
    pub dead_letter: Option<DeadLetter>,
}

impl Default for Attributes {
    fn default() -> Self {
        Self {
            visibility_timeout: Seconds::new(30),
            delay: Seconds::new(0),
            retention: Seconds::new(345_600),
            kind: QueueKind::Standard,
            dead_letter: None,
        }
    }
}

impl Attributes {
    /// The `Attributes` member of a `GetQueueAttributes` result.
    pub fn to_json(&self) -> Value {
        let mut json = json!({
            "VisibilityTimeout": self.visibility_timeout.get(),
            "DelaySeconds": self.delay.get(),
            "RetentionSeconds": self.retention.get(),
            "Fifo": self.kind.is_fifo(),
            "ContentBasedDeduplication": self.kind.content_based_deduplication(),
        });
        if let Some(dead_letter) = &self.dead_letter {
            json["DeadLetterQueue"] = json!(dead_letter.queue);
            json["MaxReceiveCount"] = json!(dead_letter.max_receive_count);
        }
        json
    }

    /// Overwrites the attributes that `update` carries and keeps the others.
    /// The caller has already validated the update.
    pub fn apply(&mut self, update: AttributeUpdate) {
        let AttributeUpdate {
            visibility_timeout,
            delay,
            retention,
            fifo,
            content_based_deduplication,
            dead_letter,
        } = update;
        self.visibility_timeout = visibility_timeout.unwrap_or(self.visibility_timeout);
        self.delay = delay.unwrap_or(self.delay);
        self.retention = retention.unwrap_or(self.retention);
        self.kind = if fifo.unwrap_or(self.kind.is_fifo()) {
            QueueKind::Fifo {
                content_based_deduplication: content_based_deduplication
                    .unwrap_or(self.kind.content_based_deduplication()),
            }
        } else {
            QueueKind::Standard
        };
        if let DeadLetterUpdate::Complete(dead_letter) = dead_letter {
            self.dead_letter = Some(dead_letter);
        }
    }
}

/// The dead-letter members of a request. Giving only one of the two is
/// representable here because SPEC section 5 reports it after other checks.
#[derive(Debug, Default)]
pub enum DeadLetterUpdate {
    #[default]
    Absent,
    Complete(DeadLetter),
    OnlyOneMember,
}

/// The attributes a request carries; absent ones are `None`.
#[derive(Debug, Default)]
pub struct AttributeUpdate {
    pub visibility_timeout: Option<Seconds>,
    pub delay: Option<Seconds>,
    pub retention: Option<Seconds>,
    pub fifo: Option<bool>,
    pub content_based_deduplication: Option<bool>,
    pub dead_letter: DeadLetterUpdate,
}

impl AttributeUpdate {
    /// Validates names, types and ranges of an `Attributes` object
    /// (SPEC section 2, check 3). `None` stands for an absent object.
    pub fn parse(attributes: Option<Params<'_>>) -> Result<Self> {
        let Some(p) = attributes else {
            return Ok(Self::default());
        };
        p.reject_unknown(&ATTRIBUTE_NAMES)?;
        let dead_letter_queue = p.string("DeadLetterQueue")?;
        let max_receive_count = p.integer("MaxReceiveCount", MAX_RECEIVE_COUNT_RANGE)?;
        Ok(Self {
            visibility_timeout: p.seconds("VisibilityTimeout", VISIBILITY_TIMEOUT_RANGE)?,
            delay: p.seconds("DelaySeconds", DELAY_RANGE)?,
            retention: p.seconds("RetentionSeconds", RETENTION_RANGE)?,
            fifo: p.boolean("Fifo")?,
            content_based_deduplication: p.boolean("ContentBasedDeduplication")?,
            dead_letter: match (dead_letter_queue, max_receive_count) {
                (Some(queue), Some(max_receive_count)) => DeadLetterUpdate::Complete(DeadLetter {
                    queue: queue.to_owned(),
                    max_receive_count,
                }),
                (None, None) => DeadLetterUpdate::Absent,
                _ => DeadLetterUpdate::OnlyOneMember,
            },
        })
    }

    /// The dead-letter configuration, which must be given completely or not
    /// at all.
    pub fn dead_letter(&self) -> Result<Option<&DeadLetter>> {
        match &self.dead_letter {
            DeadLetterUpdate::Absent => Ok(None),
            DeadLetterUpdate::Complete(dead_letter) => Ok(Some(dead_letter)),
            DeadLetterUpdate::OnlyOneMember => Err(Error::new(
                ErrorCode::InvalidParameterValue,
                "`DeadLetterQueue` and `MaxReceiveCount` must be given together",
            )),
        }
    }
}
