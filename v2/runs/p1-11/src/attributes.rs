//! Queue attributes: the full set a queue holds, and the partial set a request carries.

use serde_json::{Value, json};

use crate::error::{Error, Result};
use crate::params::Params;

const NAMES: [&str; 7] = [
    "VisibilityTimeout",
    "DelaySeconds",
    "RetentionSeconds",
    "Fifo",
    "ContentBasedDeduplication",
    "DeadLetterQueue",
    "MaxReceiveCount",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeadLetter {
    pub queue: String,
    pub max_receive_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attributes {
    pub visibility_timeout: u32,
    pub delay_seconds: u32,
    pub retention_seconds: u32,
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
    /// Overwrites the attributes that `update` carries. `dead_letter` is the
    /// update's dead-letter configuration as returned by
    /// [`AttributeUpdate::dead_letter`]; `None` leaves the current one alone.
    pub fn apply(&mut self, update: &AttributeUpdate, dead_letter: Option<DeadLetter>) {
        self.visibility_timeout = update.visibility_timeout.unwrap_or(self.visibility_timeout);
        self.delay_seconds = update.delay_seconds.unwrap_or(self.delay_seconds);
        self.retention_seconds = update.retention_seconds.unwrap_or(self.retention_seconds);
        self.fifo = update.fifo.unwrap_or(self.fifo);
        self.content_based_deduplication = update
            .content_based_deduplication
            .unwrap_or(self.content_based_deduplication);
        if dead_letter.is_some() {
            self.dead_letter = dead_letter;
        }
    }

    pub fn to_json(&self) -> Value {
        let mut attributes = json!({
            "VisibilityTimeout": self.visibility_timeout,
            "DelaySeconds": self.delay_seconds,
            "RetentionSeconds": self.retention_seconds,
            "Fifo": self.fifo,
            "ContentBasedDeduplication": self.content_based_deduplication,
        });
        if let Some(dead_letter) = &self.dead_letter {
            attributes["DeadLetterQueue"] = json!(dead_letter.queue);
            attributes["MaxReceiveCount"] = json!(dead_letter.max_receive_count);
        }
        attributes
    }
}

/// The attributes a request names; every one is optional.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AttributeUpdate {
    pub visibility_timeout: Option<u32>,
    pub delay_seconds: Option<u32>,
    pub retention_seconds: Option<u32>,
    pub fifo: Option<bool>,
    pub content_based_deduplication: Option<bool>,
    pub dead_letter_queue: Option<String>,
    pub max_receive_count: Option<u32>,
}

impl AttributeUpdate {
    /// Validates the value of every attribute and the attribute names.
    pub fn parse(attributes: Params<'_>) -> Result<Self> {
        attributes.reject_unknown(&NAMES)?;
        Ok(Self {
            visibility_timeout: attributes.integer("VisibilityTimeout", 0..=43_200)?,
            delay_seconds: attributes.integer("DelaySeconds", 0..=900)?,
            retention_seconds: attributes.integer("RetentionSeconds", 60..=1_209_600)?,
            fifo: attributes.boolean("Fifo")?,
            content_based_deduplication: attributes.boolean("ContentBasedDeduplication")?,
            dead_letter_queue: attributes.string("DeadLetterQueue")?.map(str::to_owned),
            max_receive_count: attributes.integer("MaxReceiveCount", 1..=1000)?,
        })
    }

    /// The dead-letter configuration, which must be given as a pair or not at all.
    pub fn dead_letter(&self) -> Result<Option<DeadLetter>> {
        match (&self.dead_letter_queue, self.max_receive_count) {
            (Some(queue), Some(max_receive_count)) => Ok(Some(DeadLetter {
                queue: queue.clone(),
                max_receive_count,
            })),
            (None, None) => Ok(None),
            _ => Err(Error::invalid_value(
                "DeadLetterQueue and MaxReceiveCount",
                "must be given together",
            )),
        }
    }
}
