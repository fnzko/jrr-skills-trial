//! Queue attributes (SPEC section 4) and their per-value checks.
//!
//! Two types, because the spec separates the checks: [`AttributeUpdates`] is
//! what a client sent, each present value already checked for type and range;
//! [`Attributes`] is a queue's full state, defaults applied. Rules that span
//! several attributes (dead-letter pairing, FIFO against the dead-letter
//! queue) belong to the commands, not here.

use std::ops::RangeInclusive;

use serde_json::{Map, Value};

use crate::error::Result;
use crate::params::{boolean, integer, invalid, string};

pub const VISIBILITY_TIMEOUT: &str = "VisibilityTimeout";
pub const DELAY_SECONDS: &str = "DelaySeconds";
pub const RETENTION_SECONDS: &str = "RetentionSeconds";
pub const FIFO: &str = "Fifo";
pub const CONTENT_BASED_DEDUPLICATION: &str = "ContentBasedDeduplication";
pub const DEAD_LETTER_QUEUE: &str = "DeadLetterQueue";
pub const MAX_RECEIVE_COUNT: &str = "MaxReceiveCount";

pub const VISIBILITY_TIMEOUT_RANGE: RangeInclusive<u64> = 0..=43_200;
pub const DELAY_SECONDS_RANGE: RangeInclusive<u64> = 0..=900;
pub const RETENTION_SECONDS_RANGE: RangeInclusive<u64> = 60..=1_209_600;
pub const MAX_RECEIVE_COUNT_RANGE: RangeInclusive<u64> = 1..=1_000;

/// Where a queue sends messages it can no longer deliver.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeadLetterPolicy {
    pub queue_name: String,
    pub max_receive_count: u64,
}

/// A queue's attributes with defaults applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attributes {
    pub visibility_timeout: u64,
    pub delay_seconds: u64,
    pub retention_seconds: u64,
    pub fifo: bool,
    pub content_based_deduplication: bool,
    pub dead_letter: Option<DeadLetterPolicy>,
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

/// The attributes a client named. `None` means absent (defaulted or left as is).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AttributeUpdates {
    pub visibility_timeout: Option<u64>,
    pub delay_seconds: Option<u64>,
    pub retention_seconds: Option<u64>,
    pub fifo: Option<bool>,
    pub content_based_deduplication: Option<bool>,
    pub dead_letter_queue: Option<String>,
    pub max_receive_count: Option<u64>,
}

impl AttributeUpdates {
    /// Parses an `Attributes` object. Each attribute is checked on its own:
    /// the value must be an object, every name must be one this section lists,
    /// and every present value must have the right type and range. A `null`
    /// attribute counts as absent, so it is skipped even if its name is unknown.
    pub fn parse(value: &Value) -> Result<Self> {
        let object: &Map<String, Value> = value
            .as_object()
            .ok_or_else(|| invalid("Attributes must be an object"))?;

        let mut updates = AttributeUpdates::default();
        for (name, value) in object {
            if value.is_null() {
                continue;
            }
            match name.as_str() {
                VISIBILITY_TIMEOUT => {
                    updates.visibility_timeout =
                        Some(integer(value, name, VISIBILITY_TIMEOUT_RANGE)?)
                }
                DELAY_SECONDS => {
                    updates.delay_seconds = Some(integer(value, name, DELAY_SECONDS_RANGE)?)
                }
                RETENTION_SECONDS => {
                    updates.retention_seconds = Some(integer(value, name, RETENTION_SECONDS_RANGE)?)
                }
                FIFO => updates.fifo = Some(boolean(value, name)?),
                CONTENT_BASED_DEDUPLICATION => {
                    updates.content_based_deduplication = Some(boolean(value, name)?)
                }
                DEAD_LETTER_QUEUE => {
                    updates.dead_letter_queue = Some(string(value, name)?.to_owned())
                }
                MAX_RECEIVE_COUNT => {
                    updates.max_receive_count = Some(integer(value, name, MAX_RECEIVE_COUNT_RANGE)?)
                }
                other => return Err(invalid(format!("unknown attribute {other}"))),
            }
        }
        Ok(updates)
    }

    /// The dead-letter configuration the request carries. The two members are
    /// set together or not at all, so one without the other is an error.
    pub fn dead_letter(&self) -> Result<Option<DeadLetterPolicy>> {
        match (&self.dead_letter_queue, self.max_receive_count) {
            (None, None) => Ok(None),
            (Some(queue_name), Some(max_receive_count)) => Ok(Some(DeadLetterPolicy {
                queue_name: queue_name.clone(),
                max_receive_count,
            })),
            _ => Err(invalid(format!(
                "{DEAD_LETTER_QUEUE} and {MAX_RECEIVE_COUNT} must be given together"
            ))),
        }
    }

    /// Writes every present value into `target`. `dead_letter` is the result of
    /// [`dead_letter`](Self::dead_letter), passed in so the pairing check runs once.
    pub fn apply_to(&self, target: &mut Attributes, dead_letter: Option<DeadLetterPolicy>) {
        if let Some(v) = self.visibility_timeout {
            target.visibility_timeout = v;
        }
        if let Some(v) = self.delay_seconds {
            target.delay_seconds = v;
        }
        if let Some(v) = self.retention_seconds {
            target.retention_seconds = v;
        }
        if let Some(v) = self.fifo {
            target.fifo = v;
        }
        if let Some(v) = self.content_based_deduplication {
            target.content_based_deduplication = v;
        }
        if dead_letter.is_some() {
            target.dead_letter = dead_letter;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorCode;
    use serde_json::json;

    fn parse(value: Value) -> Result<AttributeUpdates> {
        AttributeUpdates::parse(&value)
    }

    #[test]
    fn defaults_match_the_spec_table() {
        let a = Attributes::default();
        assert_eq!(a.visibility_timeout, 30);
        assert_eq!(a.delay_seconds, 0);
        assert_eq!(a.retention_seconds, 345_600);
        assert!(!a.fifo);
        assert!(!a.content_based_deduplication);
        assert!(a.dead_letter.is_none());
    }

    #[test]
    fn null_attributes_are_absent() {
        let updates = parse(json!({"DelaySeconds": null, "Unknown": null})).unwrap();
        assert_eq!(updates, AttributeUpdates::default());
    }

    #[test]
    fn unknown_attribute_names_are_rejected_case_sensitively() {
        for name in ["Bogus", "delaySeconds"] {
            let err = parse(json!({ name: 1 })).unwrap_err();
            assert_eq!(err.code, ErrorCode::InvalidParameterValue);
        }
    }

    #[test]
    fn attributes_must_be_an_object() {
        assert_eq!(
            parse(json!([])).unwrap_err().code,
            ErrorCode::InvalidParameterValue
        );
    }

    #[test]
    fn values_are_checked_against_their_ranges() {
        assert!(parse(json!({"VisibilityTimeout": 43200})).is_ok());
        assert!(parse(json!({"VisibilityTimeout": 43201})).is_err());
        assert!(parse(json!({"DelaySeconds": 900})).is_ok());
        assert!(parse(json!({"DelaySeconds": 901})).is_err());
        assert!(parse(json!({"RetentionSeconds": 59})).is_err());
        assert!(parse(json!({"RetentionSeconds": 60})).is_ok());
        assert!(parse(json!({"MaxReceiveCount": 0})).is_err());
        assert!(parse(json!({"MaxReceiveCount": 1000})).is_ok());
        assert!(parse(json!({"Fifo": "true"})).is_err());
    }

    #[test]
    fn dead_letter_members_are_paired() {
        let both = parse(json!({"DeadLetterQueue": "dlq", "MaxReceiveCount": 5})).unwrap();
        assert_eq!(
            both.dead_letter().unwrap(),
            Some(DeadLetterPolicy {
                queue_name: "dlq".to_owned(),
                max_receive_count: 5
            })
        );
        let neither = parse(json!({})).unwrap();
        assert_eq!(neither.dead_letter().unwrap(), None);
        let one = parse(json!({"MaxReceiveCount": 5})).unwrap();
        assert_eq!(
            one.dead_letter().unwrap_err().code,
            ErrorCode::InvalidParameterValue
        );
    }

    #[test]
    fn apply_leaves_absent_values_alone() {
        let mut target = Attributes {
            delay_seconds: 7,
            ..Attributes::default()
        };
        let updates = parse(json!({"VisibilityTimeout": 5})).unwrap();
        updates.apply_to(&mut target, None);
        assert_eq!(target.visibility_timeout, 5);
        assert_eq!(target.delay_seconds, 7);
    }
}
