//! Queue attributes: their types, defaults, validation and JSON form.

use serde_json::{Value, json};

use crate::error::{Error, Result};
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
            dead_letter_queue,
            max_receive_count,
        } = update;
        self.visibility_timeout = visibility_timeout.unwrap_or(self.visibility_timeout);
        self.delay_seconds = delay_seconds.unwrap_or(self.delay_seconds);
        self.retention_seconds = retention_seconds.unwrap_or(self.retention_seconds);
        self.fifo = fifo.unwrap_or(self.fifo);
        self.content_based_deduplication =
            content_based_deduplication.unwrap_or(self.content_based_deduplication);
        if let (Some(queue), Some(max_receive_count)) = (dead_letter_queue, max_receive_count) {
            self.dead_letter = Some(DeadLetter {
                queue,
                max_receive_count,
            });
        }
    }
}

/// The attributes a request carries; absent ones are `None`.
#[derive(Debug, Default)]
pub struct AttributeUpdate {
    pub visibility_timeout: Option<u64>,
    pub delay_seconds: Option<u64>,
    pub retention_seconds: Option<u64>,
    pub fifo: Option<bool>,
    pub content_based_deduplication: Option<bool>,
    pub dead_letter_queue: Option<String>,
    pub max_receive_count: Option<u64>,
}

impl AttributeUpdate {
    /// Validates names, types and ranges of an `Attributes` object
    /// (SPEC section 2, check 3). `None` stands for an absent object.
    pub fn parse(attributes: Option<Params<'_>>) -> Result<Self> {
        let Some(p) = attributes else {
            return Ok(Self::default());
        };
        p.reject_unknown(&ATTRIBUTE_NAMES)?;
        Ok(Self {
            visibility_timeout: p.integer("VisibilityTimeout", 0..=43_200)?,
            delay_seconds: p.integer("DelaySeconds", 0..=900)?,
            retention_seconds: p.integer("RetentionSeconds", 60..=1_209_600)?,
            fifo: p.boolean("Fifo")?,
            content_based_deduplication: p.boolean("ContentBasedDeduplication")?,
            dead_letter_queue: p.string("DeadLetterQueue")?.map(str::to_owned),
            max_receive_count: p.integer("MaxReceiveCount", 1..=1000)?,
        })
    }

    /// The dead-letter configuration, which must be given completely or not
    /// at all.
    pub fn dead_letter(&self) -> Result<Option<DeadLetter>> {
        match (&self.dead_letter_queue, self.max_receive_count) {
            (Some(queue), Some(max_receive_count)) => Ok(Some(DeadLetter {
                queue: queue.clone(),
                max_receive_count,
            })),
            (None, None) => Ok(None),
            _ => Err(Error::invalid_value(
                "`DeadLetterQueue` and `MaxReceiveCount` must be given together",
            )),
        }
    }
}
