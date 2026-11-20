//! Queue attributes: their types, defaults, validation and JSON form.

use serde_json::{Value, json};

use crate::error::{Error, ErrorCode, Result};
use crate::request::Params;

const ATTRIBUTE_NAMES: [&str; 7] = [
    "VisibilityTimeout",
    "DelaySeconds",
    "RetentionSeconds",
    "Fifo",
    "ContentBasedDeduplication",
    "DeadLetterQueue",
    "MaxReceiveCount",
];

/// Where messages go after too many deliveries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeadLetter {
    pub queue: String,
    pub max_receive_count: u64,
}

/// The complete attribute set of a queue.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attributes {
    pub visibility_timeout: u64,
    pub delay_seconds: u64,
    pub retention_seconds: u64,
    pub fifo: bool,
    pub content_based_deduplication: bool,
    pub dead_letter: Option<DeadLetter>,
}

impl Default for Attributes {
    fn default() -> Self {
        Self {
            visibility_timeout: 30,
            delay_seconds: 0,
            retention_seconds: 345_600,
            fifo: false,
            content_based_deduplication: false,
            dead_letter: None,
        }
    }
}

impl Attributes {
    /// The `Attributes` member of a `GetQueueAttributes` result.
    pub fn to_json(&self) -> Value {
        let mut json = json!({
            "VisibilityTimeout": self.visibility_timeout,
            "DelaySeconds": self.delay_seconds,
            "RetentionSeconds": self.retention_seconds,
            "Fifo": self.fifo,
            "ContentBasedDeduplication": self.content_based_deduplication,
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
            delay_seconds,
            retention_seconds,
            fifo,
            content_based_deduplication,
            dead_letter,
        } = update;
        self.visibility_timeout = visibility_timeout.unwrap_or(self.visibility_timeout);
        self.delay_seconds = delay_seconds.unwrap_or(self.delay_seconds);
        self.retention_seconds = retention_seconds.unwrap_or(self.retention_seconds);
        self.fifo = fifo.unwrap_or(self.fifo);
        self.content_based_deduplication =
            content_based_deduplication.unwrap_or(self.content_based_deduplication);
        match dead_letter {
            DeadLetterParams::Absent => {}
            DeadLetterParams::Complete(dead_letter) => self.dead_letter = Some(dead_letter),
            DeadLetterParams::Partial => {
                unreachable!("callers reject a partial dead-letter configuration first")
            }
        }
    }
}

/// The dead-letter attributes of a request. The two attributes only make
/// sense together, but a request may carry just one of them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum DeadLetterParams {
    #[default]
    Absent,
    Complete(DeadLetter),
    Partial,
}

/// The attributes a request carries; absent ones are `None`.
#[derive(Debug, Default)]
pub struct AttributeUpdate {
    pub visibility_timeout: Option<u64>,
    pub delay_seconds: Option<u64>,
    pub retention_seconds: Option<u64>,
    pub fifo: Option<bool>,
    pub content_based_deduplication: Option<bool>,
    pub dead_letter: DeadLetterParams,
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
        let max_receive_count = p.integer("MaxReceiveCount", 1..=1000)?;
        let dead_letter = match (dead_letter_queue, max_receive_count) {
            (Some(queue), Some(max_receive_count)) => DeadLetterParams::Complete(DeadLetter {
                queue: queue.to_owned(),
                max_receive_count,
            }),
            (None, None) => DeadLetterParams::Absent,
            _ => DeadLetterParams::Partial,
        };
        Ok(Self {
            visibility_timeout: p.integer("VisibilityTimeout", 0..=43_200)?,
            delay_seconds: p.integer("DelaySeconds", 0..=900)?,
            retention_seconds: p.integer("RetentionSeconds", 60..=1_209_600)?,
            fifo: p.boolean("Fifo")?,
            content_based_deduplication: p.boolean("ContentBasedDeduplication")?,
            dead_letter,
        })
    }

    /// The dead-letter configuration, which must be given completely or not
    /// at all.
    pub fn dead_letter(&self) -> Result<Option<&DeadLetter>> {
        match &self.dead_letter {
            DeadLetterParams::Absent => Ok(None),
            DeadLetterParams::Complete(dead_letter) => Ok(Some(dead_letter)),
            DeadLetterParams::Partial => Err(Error::new(
                ErrorCode::InvalidParameterValue,
                "`DeadLetterQueue` and `MaxReceiveCount` must be given together",
            )),
        }
    }
}
