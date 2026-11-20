//! Queue attributes: the table in section 4 of the specification.
//!
//! Two types are involved. [`AttributeInput`] is what a request said, with
//! `None` for every attribute it left out. [`QueueAttributes`] is the
//! configuration a queue actually has, with the defaults filled in.

use crate::params::Params;
use crate::{Error, ErrorCode};

pub const MAX_VISIBILITY_TIMEOUT: u64 = 43_200;
pub const MAX_DELAY_SECONDS: u64 = 900;
pub const RETENTION_SECONDS: std::ops::RangeInclusive<u64> = 60..=1_209_600;
pub const MAX_RECEIVE_COUNT: u64 = 1_000;

/// The dead-letter configuration of a queue. The spec makes
/// `DeadLetterQueue` and `MaxReceiveCount` set together or not at all, so
/// they are one value here and cannot disagree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeadLetterPolicy {
    pub queue: String,
    pub max_receive_count: u64,
}

/// The attributes a request gave, with `None` for those it left out.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AttributeInput {
    pub visibility_timeout: Option<u64>,
    pub delay_seconds: Option<u64>,
    pub retention_seconds: Option<u64>,
    pub fifo: Option<bool>,
    pub content_based_deduplication: Option<bool>,
    pub dead_letter_queue: Option<String>,
    pub max_receive_count: Option<u64>,
}

const KNOWN_ATTRIBUTES: [&str; 7] = [
    "VisibilityTimeout",
    "DelaySeconds",
    "RetentionSeconds",
    "Fifo",
    "ContentBasedDeduplication",
    "DeadLetterQueue",
    "MaxReceiveCount",
];

impl AttributeInput {
    /// Reads the `Attributes` object of a request. Each value is checked
    /// for type and range; a name the spec does not list is rejected.
    pub fn parse(attributes: Option<Params<'_>>) -> Result<Self, Error> {
        let Some(attributes) = attributes else {
            return Ok(Self::default());
        };
        for name in attributes.names() {
            if !KNOWN_ATTRIBUTES.contains(&name) {
                return Err(Error::new(
                    ErrorCode::InvalidParameterValue,
                    format!("unknown attribute {name}"),
                ));
            }
        }
        Ok(AttributeInput {
            visibility_timeout: attributes
                .optional_int("VisibilityTimeout", 0..=MAX_VISIBILITY_TIMEOUT)?,
            delay_seconds: attributes.optional_int("DelaySeconds", 0..=MAX_DELAY_SECONDS)?,
            retention_seconds: attributes.optional_int("RetentionSeconds", RETENTION_SECONDS)?,
            fifo: attributes.optional_bool("Fifo")?,
            content_based_deduplication: attributes.optional_bool("ContentBasedDeduplication")?,
            dead_letter_queue: attributes
                .optional_str("DeadLetterQueue")?
                .map(str::to_owned),
            max_receive_count: attributes.optional_int("MaxReceiveCount", 1..=MAX_RECEIVE_COUNT)?,
        })
    }

    /// The dead-letter configuration the request asks for. The spec
    /// requires `DeadLetterQueue` and `MaxReceiveCount` to come together,
    /// so one without the other is an `InvalidParameterValue`.
    pub fn dead_letter(&self) -> Result<Option<DeadLetterPolicy>, Error> {
        match (&self.dead_letter_queue, self.max_receive_count) {
            (None, None) => Ok(None),
            (Some(queue), Some(max_receive_count)) => Ok(Some(DeadLetterPolicy {
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

/// The configuration of an existing queue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueAttributes {
    pub visibility_timeout: u64,
    pub delay_seconds: u64,
    pub retention_seconds: u64,
    pub fifo: bool,
    pub content_based_deduplication: bool,
    pub dead_letter: Option<DeadLetterPolicy>,
}

impl Default for QueueAttributes {
    fn default() -> Self {
        QueueAttributes {
            visibility_timeout: 30,
            delay_seconds: 0,
            retention_seconds: 345_600,
            fifo: false,
            content_based_deduplication: false,
            dead_letter: None,
        }
    }
}

impl QueueAttributes {
    /// The configuration of a new queue: the request's values, and the
    /// defaults for those it left out. `dead_letter` is the value already
    /// read from the request by [`AttributeInput::dead_letter`].
    pub fn create(input: &AttributeInput, dead_letter: Option<DeadLetterPolicy>) -> Self {
        let mut attributes = QueueAttributes {
            fifo: input.fifo.unwrap_or(false),
            ..QueueAttributes::default()
        };
        attributes.update(input, dead_letter);
        attributes
    }

    /// Applies the attributes a request gave to an existing queue. The
    /// queue type (`Fifo`) never changes, so it is not touched here; the
    /// caller rejects requests that try to set it.
    pub fn update(&mut self, input: &AttributeInput, dead_letter: Option<DeadLetterPolicy>) {
        if let Some(value) = input.visibility_timeout {
            self.visibility_timeout = value;
        }
        if let Some(value) = input.delay_seconds {
            self.delay_seconds = value;
        }
        if let Some(value) = input.retention_seconds {
            self.retention_seconds = value;
        }
        if let Some(value) = input.content_based_deduplication {
            self.content_based_deduplication = value;
        }
        if let Some(policy) = dead_letter {
            self.dead_letter = Some(policy);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Map, Value};

    fn parse(value: Value) -> Result<AttributeInput, Error> {
        let Value::Object(map) = value else {
            panic!("object expected")
        };
        AttributeInput::parse(Some(Params::new(&map)))
    }

    fn code(value: Value) -> ErrorCode {
        parse(value).expect_err("should be rejected").code()
    }

    #[test]
    fn defaults_match_section_4() {
        let attributes = QueueAttributes::default();
        assert_eq!(attributes.visibility_timeout, 30);
        assert_eq!(attributes.delay_seconds, 0);
        assert_eq!(attributes.retention_seconds, 345_600);
        assert!(!attributes.fifo);
        assert!(!attributes.content_based_deduplication);
        assert_eq!(attributes.dead_letter, None);
    }

    #[test]
    fn create_fills_in_defaults_for_missing_attributes() {
        let input = parse(json!({"DelaySeconds": 5, "Fifo": true})).unwrap();
        let attributes = QueueAttributes::create(&input, None);
        assert_eq!(attributes.delay_seconds, 5);
        assert!(attributes.fifo);
        assert_eq!(attributes.visibility_timeout, 30);
    }

    #[test]
    fn null_attributes_are_absent() {
        let input = parse(json!({"DelaySeconds": null, "Fifo": null})).unwrap();
        assert_eq!(input, AttributeInput::default());
    }

    #[test]
    fn values_are_checked_against_their_ranges() {
        assert_eq!(
            code(json!({"VisibilityTimeout": 43201})),
            ErrorCode::InvalidParameterValue
        );
        assert_eq!(
            code(json!({"DelaySeconds": 901})),
            ErrorCode::InvalidParameterValue
        );
        assert_eq!(
            code(json!({"RetentionSeconds": 59})),
            ErrorCode::InvalidParameterValue
        );
        assert_eq!(
            code(json!({"MaxReceiveCount": 0})),
            ErrorCode::InvalidParameterValue
        );
        assert_eq!(
            code(json!({"MaxReceiveCount": 1001})),
            ErrorCode::InvalidParameterValue
        );
    }

    #[test]
    fn boundary_values_are_accepted() {
        let input = parse(json!({
            "VisibilityTimeout": 0,
            "DelaySeconds": 900,
            "RetentionSeconds": 1209600,
            "MaxReceiveCount": 1,
            "DeadLetterQueue": "dead"
        }))
        .unwrap();
        assert_eq!(input.visibility_timeout, Some(0));
        assert_eq!(input.retention_seconds, Some(1_209_600));
        assert_eq!(input.max_receive_count, Some(1));
    }

    #[test]
    fn values_must_have_the_right_json_type() {
        assert_eq!(
            code(json!({"Fifo": "true"})),
            ErrorCode::InvalidParameterValue
        );
        assert_eq!(
            code(json!({"DeadLetterQueue": 3})),
            ErrorCode::InvalidParameterValue
        );
        assert_eq!(
            code(json!({"VisibilityTimeout": 30.0})),
            ErrorCode::InvalidParameterValue
        );
    }

    #[test]
    fn unknown_attribute_names_are_rejected() {
        assert_eq!(
            code(json!({"Colour": "red"})),
            ErrorCode::InvalidParameterValue
        );
        assert_eq!(
            code(json!({"fifo": true})),
            ErrorCode::InvalidParameterValue
        );
    }

    #[test]
    fn null_unknown_attribute_is_treated_as_absent() {
        assert_eq!(
            parse(json!({"Colour": null})).unwrap(),
            AttributeInput::default()
        );
    }

    #[test]
    fn attributes_that_are_not_an_object_are_rejected() {
        let mut map = Map::new();
        map.insert("Attributes".to_owned(), json!([]));
        let attributes = Params::new(&map).optional_object("Attributes");
        assert_eq!(
            attributes.unwrap_err().code(),
            ErrorCode::InvalidParameterValue
        );
    }

    #[test]
    fn dead_letter_needs_both_members_or_neither() {
        let neither = parse(json!({})).unwrap();
        assert_eq!(neither.dead_letter().unwrap(), None);

        let both = parse(json!({"DeadLetterQueue": "dead", "MaxReceiveCount": 3})).unwrap();
        assert_eq!(
            both.dead_letter().unwrap(),
            Some(DeadLetterPolicy {
                queue: "dead".to_owned(),
                max_receive_count: 3
            })
        );

        let only_queue = parse(json!({"DeadLetterQueue": "dead"})).unwrap();
        assert_eq!(
            only_queue.dead_letter().unwrap_err().code(),
            ErrorCode::InvalidParameterValue
        );
        let only_count = parse(json!({"MaxReceiveCount": 3})).unwrap();
        assert_eq!(
            only_count.dead_letter().unwrap_err().code(),
            ErrorCode::InvalidParameterValue
        );
    }

    #[test]
    fn update_changes_only_the_attributes_it_was_given() {
        let mut attributes = QueueAttributes {
            visibility_timeout: 10,
            fifo: true,
            ..QueueAttributes::default()
        };
        let input = parse(json!({"DelaySeconds": 7})).unwrap();
        attributes.update(&input, None);
        assert_eq!(attributes.delay_seconds, 7);
        assert_eq!(attributes.visibility_timeout, 10);
        assert!(attributes.fifo, "the queue type never changes");
    }
}
