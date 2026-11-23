//! Queue attributes (SPEC section 4): parsing, defaults and the stored configuration.

use serde_json::{Map, Value};

use crate::error::{Error, ErrorCode, Result};
use crate::params::{parse_bool, parse_int};

/// Dead-letter configuration: both attributes set together.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeadLetter {
    pub queue: String,
    pub max_receive_count: u32,
}

/// The attributes of a queue, with defaults applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueConfig {
    pub visibility_timeout: u32,
    pub delay_seconds: u32,
    pub retention_seconds: u32,
    pub fifo: bool,
    pub content_based_deduplication: bool,
    pub dead_letter: Option<DeadLetter>,
}

impl Default for QueueConfig {
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

/// The attributes named in a request; `None` means "not given".
#[derive(Debug, Clone, Default, PartialEq, Eq)]
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
    /// Validates every attribute on its own (type, range, known name).
    /// Attributes whose value is `null` count as absent.
    pub fn parse(map: &Map<String, Value>) -> Result<Self> {
        let mut update = Self::default();
        for (name, value) in map.iter().filter(|(_, v)| !v.is_null()) {
            match name.as_str() {
                "VisibilityTimeout" => {
                    update.visibility_timeout = Some(int_attr(name, value, 0, 43_200)?)
                }
                "DelaySeconds" => update.delay_seconds = Some(int_attr(name, value, 0, 900)?),
                "RetentionSeconds" => {
                    update.retention_seconds = Some(int_attr(name, value, 60, 1_209_600)?)
                }
                "MaxReceiveCount" => {
                    update.max_receive_count = Some(int_attr(name, value, 1, 1000)?)
                }
                "Fifo" => update.fifo = Some(bool_attr(name, value)?),
                "ContentBasedDeduplication" => {
                    update.content_based_deduplication = Some(bool_attr(name, value)?)
                }
                "DeadLetterQueue" => {
                    let queue = value
                        .as_str()
                        .ok_or_else(|| invalid(format!("attribute {name} must be a string")))?;
                    update.dead_letter_queue = Some(queue.to_owned());
                }
                _ => return Err(invalid(format!("unknown attribute {name}"))),
            }
        }
        Ok(update)
    }

    /// `DeadLetterQueue` and `MaxReceiveCount` must come together.
    pub fn dead_letter(&self) -> Result<Option<DeadLetter>> {
        match (&self.dead_letter_queue, self.max_receive_count) {
            (Some(queue), Some(max_receive_count)) => Ok(Some(DeadLetter {
                queue: queue.clone(),
                max_receive_count,
            })),
            (None, None) => Ok(None),
            _ => Err(invalid(
                "DeadLetterQueue and MaxReceiveCount must be given together",
            )),
        }
    }

    /// Whether the request turns on content-based deduplication.
    pub fn wants_content_based_deduplication(&self) -> bool {
        self.content_based_deduplication == Some(true)
    }
}

fn invalid(message: impl Into<String>) -> Error {
    Error::new(ErrorCode::InvalidParameterValue, message)
}

fn int_attr(name: &str, value: &Value, min: u32, max: u32) -> Result<u32> {
    parse_int(value, i64::from(min), i64::from(max))
        // The range check guarantees the value fits in u32.
        .map(|n| n as u32)
        .ok_or_else(|| {
            invalid(format!(
                "attribute {name} must be an integer from {min} to {max}"
            ))
        })
}

fn bool_attr(name: &str, value: &Value) -> Result<bool> {
    parse_bool(value).ok_or_else(|| invalid(format!("attribute {name} must be a boolean")))
}
