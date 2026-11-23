//! Parsing of the `Attributes` object shared by `CreateQueue` and
//! `SetQueueAttributes`.

use crate::error::{Code, Error, Result};
use crate::params::Params;
use crate::queue::{Attributes, DeadLetter};

const NAMES: [&str; 7] = [
    "VisibilityTimeout",
    "DelaySeconds",
    "RetentionSeconds",
    "Fifo",
    "ContentBasedDeduplication",
    "DeadLetterQueue",
    "MaxReceiveCount",
];

/// The attributes a request mentions; everything else is `None`.
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
    /// Checks names, types and ranges of every attribute (not the rules that
    /// relate attributes to each other or to a queue).
    pub fn parse(attributes: &Params) -> Result<Self> {
        attributes.deny_unknown(&NAMES)?;
        Ok(AttributeUpdate {
            visibility_timeout: attributes.integer("VisibilityTimeout", 0..=43_200)?,
            delay_seconds: attributes.integer("DelaySeconds", 0..=900)?,
            retention_seconds: attributes.integer("RetentionSeconds", 60..=1_209_600)?,
            fifo: attributes.boolean("Fifo")?,
            content_based_deduplication: attributes.boolean("ContentBasedDeduplication")?,
            dead_letter_queue: attributes.string("DeadLetterQueue")?.map(str::to_owned),
            max_receive_count: attributes.integer("MaxReceiveCount", 1..=1000)?,
        })
    }

    /// Fails unless `DeadLetterQueue` and `MaxReceiveCount` are both present
    /// or both absent.
    pub fn check_dead_letter_pair(&self) -> Result<()> {
        if self.dead_letter_queue.is_some() != self.max_receive_count.is_some() {
            return Err(Error::new(
                Code::InvalidParameterValue,
                "DeadLetterQueue and MaxReceiveCount must be given together",
            ));
        }
        Ok(())
    }

    /// The dead-letter configuration, if this update carries one. Call
    /// [`check_dead_letter_pair`](Self::check_dead_letter_pair) first.
    pub fn dead_letter(&self) -> Option<DeadLetter> {
        Some(DeadLetter {
            queue: self.dead_letter_queue.clone()?,
            max_receive_count: self.max_receive_count?,
        })
    }

    /// Full attributes: what is mentioned, defaults for the rest.
    pub fn with_defaults(&self) -> Attributes {
        let defaults = Attributes::default();
        Attributes {
            visibility_timeout: self
                .visibility_timeout
                .unwrap_or(defaults.visibility_timeout),
            delay_seconds: self.delay_seconds.unwrap_or(defaults.delay_seconds),
            retention_seconds: self.retention_seconds.unwrap_or(defaults.retention_seconds),
            fifo: self.fifo.unwrap_or(defaults.fifo),
            content_based_deduplication: self
                .content_based_deduplication
                .unwrap_or(defaults.content_based_deduplication),
            dead_letter: self.dead_letter(),
        }
    }

    /// Overwrites the attributes this update mentions.
    pub fn apply_to(&self, target: &mut Attributes) {
        if let Some(v) = self.visibility_timeout {
            target.visibility_timeout = v;
        }
        if let Some(v) = self.delay_seconds {
            target.delay_seconds = v;
        }
        if let Some(v) = self.retention_seconds {
            target.retention_seconds = v;
        }
        if let Some(v) = self.content_based_deduplication {
            target.content_based_deduplication = v;
        }
        if let Some(dead_letter) = self.dead_letter() {
            target.dead_letter = Some(dead_letter);
        }
    }
}
