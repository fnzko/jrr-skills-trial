//! Queue attributes: their types, defaults, validation and JSON form.

use std::time::Duration;

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
    pub visibility_timeout: Duration,
    pub delay: Duration,
    pub retention: Duration,
    pub fifo: bool,
    pub content_based_deduplication: bool,
    pub dead_letter: Option<DeadLetter>,
}

impl Default for Attributes {
    fn default() -> Self {
        Self {
            visibility_timeout: Duration::from_secs(30),
            delay: Duration::ZERO,
            retention: Duration::from_secs(345_600),
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
            "VisibilityTimeout": self.visibility_timeout.as_secs(),
            "DelaySeconds": self.delay.as_secs(),
            "RetentionSeconds": self.retention.as_secs(),
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
            delay,
            retention,
            fifo,
            content_based_deduplication,
            dead_letter_queue,
            max_receive_count,
        } = update;
        self.visibility_timeout = visibility_timeout.unwrap_or(self.visibility_timeout);
        self.delay = delay.unwrap_or(self.delay);
        self.retention = retention.unwrap_or(self.retention);
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
    pub visibility_timeout: Option<Duration>,
    pub delay: Option<Duration>,
    pub retention: Option<Duration>,
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
            visibility_timeout: p.seconds("VisibilityTimeout", 0..=43_200)?,
            delay: p.seconds("DelaySeconds", 0..=900)?,
            retention: p.seconds("RetentionSeconds", 60..=1_209_600)?,
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
            _ => Err(Error::new(
                ErrorCode::InvalidParameterValue,
                "`DeadLetterQueue` and `MaxReceiveCount` must be given together",
            )),
        }
    }
}
