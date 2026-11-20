//! Queue attributes and the request forms that set them (SPEC.md section 4).

use crate::{Error, ErrorCode};

/// The configuration of one queue, with defaults already applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attributes {
    pub visibility_timeout: u32,
    pub delay_seconds: u32,
    pub retention_seconds: u32,
    pub fifo: bool,
    pub content_based_deduplication: bool,
    /// Both dead-letter members are set or both are unset, so they share one option.
    pub dead_letter: Option<DeadLetter>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeadLetter {
    pub queue: String,
    pub max_receive_count: u32,
}

impl Default for Attributes {
    fn default() -> Self {
        Attributes {
            visibility_timeout: 30,
            delay_seconds: 0,
            retention_seconds: 345_600,
            fifo: false,
            content_based_deduplication: false,
            dead_letter: None,
        }
    }
}

/// The attributes a request gives. `None` means the request did not set it,
/// which is different from setting it to the default.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AttributeChanges {
    pub visibility_timeout: Option<u32>,
    pub delay_seconds: Option<u32>,
    pub retention_seconds: Option<u32>,
    pub fifo: Option<bool>,
    pub content_based_deduplication: Option<bool>,
    pub dead_letter_queue: Option<String>,
    pub max_receive_count: Option<u32>,
}

impl AttributeChanges {
    /// The dead-letter configuration the request gives: `None` when neither
    /// member is present, an error when only one of them is.
    pub fn dead_letter(&self) -> Result<Option<DeadLetter>, Error> {
        match (&self.dead_letter_queue, self.max_receive_count) {
            (None, None) => Ok(None),
            (Some(queue), Some(max_receive_count)) => Ok(Some(DeadLetter {
                queue: queue.clone(),
                max_receive_count,
            })),
            _ => Err(Error::new(
                ErrorCode::InvalidParameterValue,
                "DeadLetterQueue and MaxReceiveCount must be given together",
            )),
        }
    }
}

impl Attributes {
    /// Overwrites every attribute the request set and keeps the others.
    /// The dead-letter configuration is replaced only when `dead_letter` is given.
    pub fn apply(&mut self, changes: &AttributeChanges, dead_letter: Option<DeadLetter>) {
        if let Some(value) = changes.visibility_timeout {
            self.visibility_timeout = value;
        }
        if let Some(value) = changes.delay_seconds {
            self.delay_seconds = value;
        }
        if let Some(value) = changes.retention_seconds {
            self.retention_seconds = value;
        }
        if let Some(value) = changes.fifo {
            self.fifo = value;
        }
        if let Some(value) = changes.content_based_deduplication {
            self.content_based_deduplication = value;
        }
        if dead_letter.is_some() {
            self.dead_letter = dead_letter;
        }
    }
}
