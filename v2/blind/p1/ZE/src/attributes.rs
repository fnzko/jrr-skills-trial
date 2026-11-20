//! Queue attributes: the full set a queue holds and the partial set a request carries.

use crate::error::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeadLetterConfig {
    pub queue: String,
    pub max_receive_count: u64,
}

/// The attribute values of one queue, defaults filled in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueConfig {
    pub visibility_timeout: u64,
    pub delay_seconds: u64,
    pub retention_seconds: u64,
    pub fifo: bool,
    pub content_based_deduplication: bool,
    pub dead_letter: Option<DeadLetterConfig>,
}

/// The attributes named in a request; absent ones are left alone or defaulted.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
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
    /// The dead-letter configuration the request names. The two attributes
    /// must come together.
    pub fn dead_letter(&self) -> Result<Option<DeadLetterConfig>, Error> {
        match (&self.dead_letter_queue, self.max_receive_count) {
            (Some(queue), Some(max_receive_count)) => Ok(Some(DeadLetterConfig {
                queue: queue.clone(),
                max_receive_count,
            })),
            (None, None) => Ok(None),
            _ => Err(Error::invalid_value(
                "DeadLetterQueue and MaxReceiveCount must be given together",
            )),
        }
    }

    /// The configuration of a new queue: given attributes, defaults for the rest.
    pub fn into_config(self, dead_letter: Option<DeadLetterConfig>) -> QueueConfig {
        QueueConfig {
            visibility_timeout: self.visibility_timeout.unwrap_or(30),
            delay_seconds: self.delay_seconds.unwrap_or(0),
            retention_seconds: self.retention_seconds.unwrap_or(345_600),
            fifo: self.fifo.unwrap_or(false),
            content_based_deduplication: self.content_based_deduplication.unwrap_or(false),
            dead_letter,
        }
    }

    /// Overwrites the attributes of `config` that the request names. `Fifo`
    /// is never changed: callers reject it first.
    pub fn apply(self, config: &mut QueueConfig, dead_letter: Option<DeadLetterConfig>) {
        let Self {
            visibility_timeout,
            delay_seconds,
            retention_seconds,
            fifo: _,
            content_based_deduplication,
            dead_letter_queue: _,
            max_receive_count: _,
        } = self;
        if let Some(value) = visibility_timeout {
            config.visibility_timeout = value;
        }
        if let Some(value) = delay_seconds {
            config.delay_seconds = value;
        }
        if let Some(value) = retention_seconds {
            config.retention_seconds = value;
        }
        if let Some(value) = content_based_deduplication {
            config.content_based_deduplication = value;
        }
        if dead_letter.is_some() {
            config.dead_letter = dead_letter;
        }
    }
}
