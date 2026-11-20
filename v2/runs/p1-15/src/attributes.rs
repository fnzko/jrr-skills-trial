//! Queue attributes: the stored configuration and the partial form a request carries.

use crate::error::{Error, ErrorCode, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeadLetter {
    pub queue: String,
    pub max_receive_count: u32,
}

/// The attributes of a queue, after defaults.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub visibility_timeout: u32,
    pub delay_seconds: u32,
    pub retention_seconds: u32,
    pub fifo: bool,
    pub content_based_deduplication: bool,
    pub dead_letter: Option<DeadLetter>,
}

impl Default for Config {
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

impl Config {
    /// Overwrites the attributes that `patch` carries. `Fifo` is never touched here:
    /// the type of a queue is fixed when it is created.
    pub fn apply(&mut self, patch: &AttributePatch, dead_letter: Option<DeadLetter>) {
        if let Some(v) = patch.visibility_timeout {
            self.visibility_timeout = v;
        }
        if let Some(v) = patch.delay_seconds {
            self.delay_seconds = v;
        }
        if let Some(v) = patch.retention_seconds {
            self.retention_seconds = v;
        }
        if let Some(v) = patch.content_based_deduplication {
            self.content_based_deduplication = v;
        }
        if dead_letter.is_some() {
            self.dead_letter = dead_letter;
        }
    }
}

/// The attributes present in a request, each already checked against its own type and range.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AttributePatch {
    pub visibility_timeout: Option<u32>,
    pub delay_seconds: Option<u32>,
    pub retention_seconds: Option<u32>,
    pub fifo: Option<bool>,
    pub content_based_deduplication: Option<bool>,
    pub dead_letter_queue: Option<String>,
    pub max_receive_count: Option<u32>,
}

impl AttributePatch {
    /// The dead-letter configuration of the request. Both attributes must be present, or neither.
    pub fn dead_letter(&self) -> Result<Option<DeadLetter>> {
        match (&self.dead_letter_queue, self.max_receive_count) {
            (Some(queue), Some(max_receive_count)) => Ok(Some(DeadLetter {
                queue: queue.clone(),
                max_receive_count,
            })),
            (None, None) => Ok(None),
            _ => Err(Error::new(
                ErrorCode::InvalidParameterValue,
                "DeadLetterQueue and MaxReceiveCount must be given together",
            )),
        }
    }
}
