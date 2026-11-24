//! Queue attributes: their types, defaults, validation and JSON form.

use serde_json::{Value, json};

use crate::error::{Error, Result};
use crate::request::Params;
use crate::time::Seconds;

pub const MAX_VISIBILITY_TIMEOUT: u64 = 43_200;
pub const MAX_DELAY_SECONDS: u64 = 900;
const MIN_RETENTION_SECONDS: u64 = 60;
const MAX_RETENTION_SECONDS: u64 = 1_209_600;
const MAX_RECEIVE_COUNT_LIMIT: u64 = 1000;

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
    pub visibility_timeout: Seconds,
    pub delay: Seconds,
    pub retention: Seconds,
    pub fifo: bool,
    pub content_based_deduplication: bool,
    pub dead_letter: Option<DeadLetter>,
}

impl Default for Attributes {
    fn default() -> Self {
        Self {
            visibility_timeout: Seconds::new(30),
            delay: Seconds::ZERO,
            retention: Seconds::new(345_600),
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
            "VisibilityTimeout": Value::from(self.visibility_timeout),
            "DelaySeconds": Value::from(self.delay),
            "RetentionSeconds": Value::from(self.retention),
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
    /// `dead_letter` is the result of [`AttributeUpdate::dead_letter`] for the
    /// same update; the caller has already validated both.
    pub fn apply(&mut self, update: AttributeUpdate, dead_letter: Option<DeadLetter>) {
        let AttributeUpdate {
            visibility_timeout,
            delay,
            retention,
            fifo,
            content_based_deduplication,
            dead_letter_queue: _,
            max_receive_count: _,
        } = update;
        self.visibility_timeout = visibility_timeout.unwrap_or(self.visibility_timeout);
        self.delay = delay.unwrap_or(self.delay);
        self.retention = retention.unwrap_or(self.retention);
        self.fifo = fifo.unwrap_or(self.fifo);
        self.content_based_deduplication =
            content_based_deduplication.unwrap_or(self.content_based_deduplication);
        if dead_letter.is_some() {
            self.dead_letter = dead_letter;
        }
    }
}

/// The attributes a request carries; absent ones are `None`.
#[derive(Debug, Default)]
pub struct AttributeUpdate {
    pub visibility_timeout: Option<Seconds>,
    pub delay: Option<Seconds>,
    pub retention: Option<Seconds>,
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
            visibility_timeout: p.seconds("VisibilityTimeout", 0..=MAX_VISIBILITY_TIMEOUT)?,
            delay: p.seconds("DelaySeconds", 0..=MAX_DELAY_SECONDS)?,
            retention: p.seconds(
                "RetentionSeconds",
                MIN_RETENTION_SECONDS..=MAX_RETENTION_SECONDS,
            )?,
            fifo: p.boolean("Fifo")?,
            content_based_deduplication: p.boolean("ContentBasedDeduplication")?,
            dead_letter_queue: p.string("DeadLetterQueue")?.map(str::to_owned),
            max_receive_count: p.integer("MaxReceiveCount", 1..=MAX_RECEIVE_COUNT_LIMIT)?,
        })
    }

    /// Fails if the update turns on content-based deduplication for a queue
    /// that is not FIFO (`fifo`).
    pub fn check_content_based_deduplication(&self, fifo: bool) -> Result<()> {
        if self.content_based_deduplication == Some(true) && !fifo {
            return Err(Error::invalid_parameter(
                "`ContentBasedDeduplication` requires a FIFO queue",
            ));
        }
        Ok(())
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
            _ => Err(Error::invalid_parameter(
                "`DeadLetterQueue` and `MaxReceiveCount` must be given together",
            )),
        }
    }
}
