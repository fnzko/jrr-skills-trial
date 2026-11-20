//! Queue attributes: the stored values and the partial form sent by clients.

use serde_json::{Map, Value, json};

use crate::error::{Error, ErrorCode};
use crate::params::{boolean, integer, invalid, string};

/// Where messages go after too many deliveries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeadLetter {
    pub queue: String,
    pub max_receive_count: u64,
}

/// The attributes of a queue, with all defaults applied.
#[derive(Debug, Clone, PartialEq, Eq)]
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
    /// Overwrites the attributes that `patch` carries.
    ///
    /// `dead_letter` is the result of [`AttributePatch::dead_letter`]; it is
    /// passed in because the caller checks it at its own point in the rule
    /// order.
    pub fn apply(&mut self, patch: &AttributePatch, dead_letter: Option<DeadLetter>) {
        if let Some(value) = patch.visibility_timeout {
            self.visibility_timeout = value;
        }
        if let Some(value) = patch.delay_seconds {
            self.delay_seconds = value;
        }
        if let Some(value) = patch.retention_seconds {
            self.retention_seconds = value;
        }
        if let Some(value) = patch.fifo {
            self.fifo = value;
        }
        if let Some(value) = patch.content_based_deduplication {
            self.content_based_deduplication = value;
        }
        if dead_letter.is_some() {
            self.dead_letter = dead_letter;
        }
    }

    /// The `Attributes` member of a `GetQueueAttributes` result.
    pub fn to_json(&self) -> Value {
        let mut members = Map::new();
        members.insert("VisibilityTimeout".into(), json!(self.visibility_timeout));
        members.insert("DelaySeconds".into(), json!(self.delay_seconds));
        members.insert("RetentionSeconds".into(), json!(self.retention_seconds));
        members.insert("Fifo".into(), json!(self.fifo));
        members.insert(
            "ContentBasedDeduplication".into(),
            json!(self.content_based_deduplication),
        );
        if let Some(dead_letter) = &self.dead_letter {
            members.insert("DeadLetterQueue".into(), json!(dead_letter.queue));
            members.insert(
                "MaxReceiveCount".into(),
                json!(dead_letter.max_receive_count),
            );
        }
        Value::Object(members)
    }
}

/// The attributes present in a request; every member is checked for type and
/// range but not yet for the rules that relate attributes to each other.
#[derive(Debug, Default)]
pub struct AttributePatch {
    pub visibility_timeout: Option<u64>,
    pub delay_seconds: Option<u64>,
    pub retention_seconds: Option<u64>,
    pub fifo: Option<bool>,
    pub content_based_deduplication: Option<bool>,
    dead_letter_queue: Option<String>,
    max_receive_count: Option<u64>,
}

impl AttributePatch {
    /// Checks the `Attributes` parameter: an object of known attributes.
    /// `null` members are absent.
    pub fn parse(value: &Value) -> Result<Self, Error> {
        let Value::Object(members) = value else {
            return Err(invalid("Attributes", "must be an object"));
        };
        let mut patch = Self::default();
        for (name, value) in members {
            let value = Some(value).filter(|value| !value.is_null());
            let name = name.as_str();
            match name {
                "VisibilityTimeout" => {
                    patch.visibility_timeout =
                        value.map(|v| integer(name, v, 0..=43_200)).transpose()?;
                }
                "DelaySeconds" => {
                    patch.delay_seconds = value.map(|v| integer(name, v, 0..=900)).transpose()?;
                }
                "RetentionSeconds" => {
                    patch.retention_seconds = value
                        .map(|v| integer(name, v, 60..=1_209_600))
                        .transpose()?;
                }
                "Fifo" => patch.fifo = value.map(|v| boolean(name, v)).transpose()?,
                "ContentBasedDeduplication" => {
                    patch.content_based_deduplication =
                        value.map(|v| boolean(name, v)).transpose()?;
                }
                "DeadLetterQueue" => {
                    patch.dead_letter_queue = value.map(|v| string(name, v)).transpose()?;
                }
                "MaxReceiveCount" => {
                    patch.max_receive_count =
                        value.map(|v| integer(name, v, 1..=1000)).transpose()?;
                }
                _ => {
                    return Err(invalid(
                        "Attributes",
                        &format!("has unknown attribute `{name}`"),
                    ));
                }
            }
        }
        Ok(patch)
    }

    /// The dead-letter configuration of the patch: both attributes or neither.
    pub fn dead_letter(&self) -> Result<Option<DeadLetter>, Error> {
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
