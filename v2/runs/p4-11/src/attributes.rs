//! Queue attributes of section 4: the complete settings of a queue, and the
//! partial updates a request carries.

use serde_json::{Map, Value};

use crate::error::Result;
use crate::types::QueueName;
use crate::wire;

pub const DEFAULT_VISIBILITY_TIMEOUT: u64 = 30;
pub const DEFAULT_RETENTION_SECONDS: u64 = 345_600;
const MAX_VISIBILITY_TIMEOUT: u64 = 43_200;
const MAX_DELAY_SECONDS: u64 = 900;
const MIN_RETENTION_SECONDS: u64 = 60;
const MAX_RETENTION_SECONDS: u64 = 1_209_600;
const MAX_RECEIVE_COUNT: u64 = 1_000;

/// Whether a queue keeps arrival order per group and deduplicates sends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueKind {
    Standard,
    Fifo,
}

/// Where a queue sends messages that were received too often.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeadLetter {
    pub queue: QueueName,
    pub max_receive_count: u32,
}

/// The complete attributes of a queue, with the defaults of section 4 applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attributes {
    pub kind: QueueKind,
    pub visibility_timeout: u64,
    pub delay_seconds: u64,
    pub retention_seconds: u64,
    pub content_based_deduplication: bool,
    pub dead_letter: Option<DeadLetter>,
}

impl Default for Attributes {
    fn default() -> Self {
        Attributes {
            kind: QueueKind::Standard,
            visibility_timeout: DEFAULT_VISIBILITY_TIMEOUT,
            delay_seconds: 0,
            retention_seconds: DEFAULT_RETENTION_SECONDS,
            content_based_deduplication: false,
            dead_letter: None,
        }
    }
}

impl Attributes {
    /// The members of `GetQueueAttributes`. The dead-letter members appear
    /// only when the configuration is set.
    pub fn to_json(&self) -> Map<String, Value> {
        let mut members = Map::new();
        members.insert("VisibilityTimeout".into(), self.visibility_timeout.into());
        members.insert("DelaySeconds".into(), self.delay_seconds.into());
        members.insert("RetentionSeconds".into(), self.retention_seconds.into());
        members.insert("Fifo".into(), (self.kind == QueueKind::Fifo).into());
        members.insert(
            "ContentBasedDeduplication".into(),
            self.content_based_deduplication.into(),
        );
        if let Some(dead_letter) = &self.dead_letter {
            members.insert("DeadLetterQueue".into(), dead_letter.queue.as_str().into());
            members.insert(
                "MaxReceiveCount".into(),
                dead_letter.max_receive_count.into(),
            );
        }
        members
    }
}

/// The attributes a request names. A field is `None` when the request did not
/// give it, so `SetQueueAttributes` can keep the current value.
///
/// The dead-letter pair is kept as the raw name and count: whether both are
/// given, and whether the named queue exists, are checked by the broker in
/// the order section 5 gives.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AttributeUpdates {
    pub visibility_timeout: Option<u64>,
    pub delay_seconds: Option<u64>,
    pub retention_seconds: Option<u64>,
    pub fifo: Option<bool>,
    pub content_based_deduplication: Option<bool>,
    pub dead_letter_queue: Option<String>,
    pub max_receive_count: Option<u32>,
}

impl AttributeUpdates {
    /// Parses the `Attributes` object of a request. Members with the value
    /// `null` count as absent; an unknown name is an error.
    pub fn from_json(value: &Value) -> Result<Self> {
        let members = value
            .as_object()
            .ok_or_else(|| wire::invalid("Attributes must be an object"))?;
        let mut updates = AttributeUpdates::default();
        for (name, value) in members {
            if value.is_null() {
                continue;
            }
            match name.as_str() {
                "VisibilityTimeout" => {
                    let number = wire::integer_in(value, name, 0, MAX_VISIBILITY_TIMEOUT)?;
                    updates.visibility_timeout = Some(number);
                }
                "DelaySeconds" => {
                    updates.delay_seconds =
                        Some(wire::integer_in(value, name, 0, MAX_DELAY_SECONDS)?);
                }
                "RetentionSeconds" => {
                    let number = wire::integer_in(
                        value,
                        name,
                        MIN_RETENTION_SECONDS,
                        MAX_RETENTION_SECONDS,
                    )?;
                    updates.retention_seconds = Some(number);
                }
                "Fifo" => updates.fifo = Some(wire::boolean(value, name)?),
                "ContentBasedDeduplication" => {
                    updates.content_based_deduplication = Some(wire::boolean(value, name)?);
                }
                "DeadLetterQueue" => {
                    updates.dead_letter_queue = Some(wire::string(value, name)?.to_owned());
                }
                "MaxReceiveCount" => {
                    let number = wire::integer_in(value, name, 1, MAX_RECEIVE_COUNT)?;
                    let count = u32::try_from(number)
                        .map_err(|_| wire::invalid(format!("{name} is out of range")))?;
                    updates.max_receive_count = Some(count);
                }
                _ => return Err(wire::invalid(format!("unknown attribute {name}"))),
            }
        }
        Ok(updates)
    }
}

impl AttributeUpdates {
    /// The dead-letter pair this request names. The two members come together:
    /// giving only one is InvalidParameterValue.
    pub fn dead_letter_request(&self) -> Result<Option<(&str, u32)>> {
        match (&self.dead_letter_queue, self.max_receive_count) {
            (Some(queue), Some(count)) => Ok(Some((queue.as_str(), count))),
            (None, None) => Ok(None),
            _ => Err(wire::invalid(
                "DeadLetterQueue and MaxReceiveCount must be given together",
            )),
        }
    }

    /// The attributes that result from applying this request to `base`. Members
    /// the request does not give keep their value in `base`. The dead-letter
    /// configuration is `dead_letter` as the caller resolved it.
    pub fn apply_to(&self, base: &Attributes, dead_letter: Option<DeadLetter>) -> Attributes {
        let kind = match self.fifo {
            Some(true) => QueueKind::Fifo,
            Some(false) => QueueKind::Standard,
            None => base.kind,
        };
        Attributes {
            kind,
            visibility_timeout: self.visibility_timeout.unwrap_or(base.visibility_timeout),
            delay_seconds: self.delay_seconds.unwrap_or(base.delay_seconds),
            retention_seconds: self.retention_seconds.unwrap_or(base.retention_seconds),
            content_based_deduplication: self
                .content_based_deduplication
                .unwrap_or(base.content_based_deduplication),
            dead_letter,
        }
    }

    /// Whether `CreateQueue` with this request finds a queue that already
    /// matches it: every attribute, with defaults for the absent ones, equal to
    /// `existing`. The dead-letter pair is compared by name, so a name that
    /// does not match the queue's configuration is a difference.
    pub fn matches_existing(
        &self,
        existing: &Attributes,
        dead_letter: Option<(&str, u32)>,
    ) -> bool {
        let wanted = self.apply_to(&Attributes::default(), None);
        let current_dead_letter = existing
            .dead_letter
            .as_ref()
            .map(|dead_letter| (dead_letter.queue.as_str(), dead_letter.max_receive_count));
        wanted.kind == existing.kind
            && wanted.visibility_timeout == existing.visibility_timeout
            && wanted.delay_seconds == existing.delay_seconds
            && wanted.retention_seconds == existing.retention_seconds
            && wanted.content_based_deduplication == existing.content_based_deduplication
            && current_dead_letter == dead_letter
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorCode;
    use serde_json::json;

    fn parse(value: Value) -> Result<AttributeUpdates> {
        AttributeUpdates::from_json(&value)
    }

    #[test]
    fn defaults_match_section_4() {
        let attributes = Attributes::default();
        assert_eq!(attributes.kind, QueueKind::Standard);
        assert_eq!(attributes.visibility_timeout, 30);
        assert_eq!(attributes.delay_seconds, 0);
        assert_eq!(attributes.retention_seconds, 345_600);
        assert!(!attributes.content_based_deduplication);
        assert_eq!(attributes.dead_letter, None);
    }

    #[test]
    fn null_members_are_absent() {
        let updates = parse(json!({"VisibilityTimeout": null, "Fifo": null})).unwrap();
        assert_eq!(updates, AttributeUpdates::default());
    }

    #[test]
    fn present_members_are_parsed_with_their_types() {
        let updates = parse(json!({
            "VisibilityTimeout": 0,
            "DelaySeconds": 900,
            "RetentionSeconds": 60,
            "Fifo": true,
            "ContentBasedDeduplication": false,
            "DeadLetterQueue": "dead",
            "MaxReceiveCount": 1000
        }))
        .unwrap();
        assert_eq!(
            updates,
            AttributeUpdates {
                visibility_timeout: Some(0),
                delay_seconds: Some(900),
                retention_seconds: Some(60),
                fifo: Some(true),
                content_based_deduplication: Some(false),
                dead_letter_queue: Some("dead".to_owned()),
                max_receive_count: Some(1000),
            }
        );
    }

    #[test]
    fn out_of_range_values_are_rejected() {
        let cases = [
            json!({"VisibilityTimeout": 43201}),
            json!({"DelaySeconds": 901}),
            json!({"RetentionSeconds": 59}),
            json!({"RetentionSeconds": 1209601}),
            json!({"MaxReceiveCount": 0}),
            json!({"MaxReceiveCount": 1001}),
        ];
        for case in cases {
            let error = parse(case.clone()).unwrap_err();
            assert_eq!(
                error.code(),
                ErrorCode::InvalidParameterValue,
                "case {case}"
            );
        }
    }

    #[test]
    fn wrong_types_and_unknown_names_are_rejected() {
        let cases = [
            json!({"DelaySeconds": "5"}),
            json!({"Fifo": "true"}),
            json!({"DeadLetterQueue": 3}),
            json!({"Colour": "red"}),
            json!({"fifo": true}),
        ];
        for case in cases {
            let error = parse(case.clone()).unwrap_err();
            assert_eq!(
                error.code(),
                ErrorCode::InvalidParameterValue,
                "case {case}"
            );
        }
    }

    #[test]
    fn attributes_that_are_not_an_object_are_rejected() {
        assert_eq!(
            parse(json!([])).unwrap_err().code(),
            ErrorCode::InvalidParameterValue
        );
        assert_eq!(
            parse(json!("x")).unwrap_err().code(),
            ErrorCode::InvalidParameterValue
        );
    }

    #[test]
    fn to_json_omits_dead_letter_members_when_unset() {
        let members = Attributes::default().to_json();
        assert_eq!(
            Value::Object(members),
            json!({
                "VisibilityTimeout": 30,
                "DelaySeconds": 0,
                "RetentionSeconds": 345600,
                "Fifo": false,
                "ContentBasedDeduplication": false
            })
        );
    }

    #[test]
    fn to_json_includes_dead_letter_members_when_set() {
        let attributes = Attributes {
            dead_letter: Some(DeadLetter {
                queue: QueueName::parse("dead").unwrap(),
                max_receive_count: 5,
            }),
            ..Attributes::default()
        };
        let members = attributes.to_json();
        assert_eq!(members["DeadLetterQueue"], "dead");
        assert_eq!(members["MaxReceiveCount"], 5);
    }

    #[test]
    fn dead_letter_pair_needs_both_members_or_neither() {
        let both = AttributeUpdates {
            dead_letter_queue: Some("dead".into()),
            max_receive_count: Some(3),
            ..AttributeUpdates::default()
        };
        assert_eq!(both.dead_letter_request().unwrap(), Some(("dead", 3)));
        assert_eq!(
            AttributeUpdates::default().dead_letter_request().unwrap(),
            None
        );
        let only_name = AttributeUpdates {
            dead_letter_queue: Some("dead".into()),
            ..AttributeUpdates::default()
        };
        assert_eq!(
            only_name.dead_letter_request().unwrap_err().code(),
            ErrorCode::InvalidParameterValue
        );
    }

    #[test]
    fn apply_to_keeps_what_the_request_does_not_give() {
        let base = Attributes {
            kind: QueueKind::Fifo,
            visibility_timeout: 5,
            delay_seconds: 6,
            retention_seconds: 7,
            content_based_deduplication: true,
            dead_letter: None,
        };
        let updates = AttributeUpdates {
            visibility_timeout: Some(9),
            ..AttributeUpdates::default()
        };
        assert_eq!(
            updates.apply_to(&base, base.dead_letter.clone()),
            Attributes {
                visibility_timeout: 9,
                ..base.clone()
            }
        );
    }

    #[test]
    fn matches_existing_compares_with_defaults_for_absent_members() {
        let existing = Attributes::default();
        assert!(AttributeUpdates::default().matches_existing(&existing, None));
        let different = AttributeUpdates {
            delay_seconds: Some(1),
            ..AttributeUpdates::default()
        };
        assert!(!different.matches_existing(&existing, None));
    }

    #[test]
    fn matches_existing_treats_an_unset_dead_letter_as_a_value() {
        let existing = Attributes {
            dead_letter: Some(DeadLetter {
                queue: QueueName::parse("dead").unwrap(),
                max_receive_count: 3,
            }),
            ..Attributes::default()
        };
        let request = AttributeUpdates::default();
        assert!(!request.matches_existing(&existing, None));
        assert!(request.matches_existing(&existing, Some(("dead", 3))));
        assert!(!request.matches_existing(&Attributes::default(), Some(("dead", 3))));
    }
}
