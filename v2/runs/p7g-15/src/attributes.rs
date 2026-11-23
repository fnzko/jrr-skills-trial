//! Queue attributes: the stored form and the form a request carries.

use std::ops::RangeInclusive;

use serde::ser::SerializeStruct;
use serde::{Serialize, Serializer};
use serde_json::{Map, Value};

use crate::error::{Error, ErrorCode};
use crate::ids::QueueName;
use crate::params::{boolean, integer, string};
use crate::time::Seconds;

/// Range of every `VisibilityTimeout` parameter and attribute.
pub const VISIBILITY_TIMEOUT: RangeInclusive<u32> = 0..=43_200;
/// Range of every `DelaySeconds` parameter and attribute.
pub const DELAY_SECONDS: RangeInclusive<u32> = 0..=900;
const RETENTION_SECONDS: RangeInclusive<u32> = 60..=1_209_600;
const MAX_RECEIVE_COUNT: RangeInclusive<u32> = 1..=1000;

const DEFAULT_VISIBILITY_TIMEOUT: Seconds = Seconds(30);
const DEFAULT_DELAY_SECONDS: Seconds = Seconds(0);
const DEFAULT_RETENTION_SECONDS: Seconds = Seconds(345_600);

/// Standard or FIFO. Content-based deduplication only exists on FIFO queues.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A standard queue.
    Standard,
    /// A FIFO queue.
    Fifo {
        /// Whether the body is the deduplication id when none is given.
        content_based_deduplication: bool,
    },
}

impl Serialize for Kind {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let (fifo, content_based) = match *self {
            Self::Standard => (false, false),
            Self::Fifo {
                content_based_deduplication,
            } => (true, content_based_deduplication),
        };
        let mut state = serializer.serialize_struct("Kind", 2)?;
        state.serialize_field("Fifo", &fifo)?;
        state.serialize_field("ContentBasedDeduplication", &content_based)?;
        state.end()
    }
}

/// Where messages go after too many deliveries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct DeadLetter {
    /// The dead-letter queue; it exists as long as this value does.
    #[serde(rename = "DeadLetterQueue")]
    pub queue: QueueName,
    /// Deliveries after which a message is moved.
    pub max_receive_count: u32,
}

/// The settings of one queue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Attributes {
    /// Default visibility timeout of later deliveries.
    pub visibility_timeout: Seconds,
    /// Default delay of later sends.
    pub delay_seconds: Seconds,
    /// How long a message lives, counted from `SentAt`.
    pub retention_seconds: Seconds,
    /// Queue type and its dependent setting.
    #[serde(flatten)]
    pub kind: Kind,
    /// Dead-letter configuration, when set.
    #[serde(flatten)]
    pub dead_letter: Option<DeadLetter>,
}

/// `DeadLetterQueue` and `MaxReceiveCount` as a request names them.
#[derive(Debug, Clone, Copy)]
pub struct DeadLetterSpec<'a> {
    /// The requested dead-letter queue; it may not exist.
    pub queue: &'a str,
    /// The requested receive limit.
    pub max_receive_count: u32,
}

/// The attributes a request names; the rest are left as they are.
#[derive(Debug, Default)]
pub struct AttributeUpdates {
    visibility_timeout: Option<Seconds>,
    delay_seconds: Option<Seconds>,
    retention_seconds: Option<Seconds>,
    fifo: Option<bool>,
    content_based_deduplication: Option<bool>,
    dead_letter_queue: Option<String>,
    max_receive_count: Option<u32>,
}

impl AttributeUpdates {
    /// Checks every attribute on its own: name, type and range.
    pub fn parse(object: &Map<String, Value>) -> Result<Self, Error> {
        let mut updates = Self::default();
        for (name, value) in object.iter().filter(|(_, value)| !value.is_null()) {
            match name.as_str() {
                "VisibilityTimeout" => {
                    updates.visibility_timeout =
                        Some(Seconds(integer(name, value, &VISIBILITY_TIMEOUT)?));
                }
                "DelaySeconds" => {
                    updates.delay_seconds = Some(Seconds(integer(name, value, &DELAY_SECONDS)?));
                }
                "RetentionSeconds" => {
                    updates.retention_seconds =
                        Some(Seconds(integer(name, value, &RETENTION_SECONDS)?));
                }
                "Fifo" => updates.fifo = Some(boolean(name, value)?),
                "ContentBasedDeduplication" => {
                    updates.content_based_deduplication = Some(boolean(name, value)?);
                }
                "DeadLetterQueue" => {
                    updates.dead_letter_queue = Some(string(name, value)?.to_owned());
                }
                "MaxReceiveCount" => {
                    updates.max_receive_count = Some(integer(name, value, &MAX_RECEIVE_COUNT)?);
                }
                _ => {
                    return Err(Error::invalid(name, "is not a queue attribute"));
                }
            }
        }
        Ok(updates)
    }

    /// The `Fifo` attribute, if the request names it.
    pub fn fifo(&self) -> Option<bool> {
        self.fifo
    }

    /// The `ContentBasedDeduplication` attribute, if the request names it.
    pub fn content_based_deduplication(&self) -> Option<bool> {
        self.content_based_deduplication
    }

    /// The dead-letter configuration, which must be given whole or not at all.
    pub fn dead_letter(&self) -> Result<Option<DeadLetterSpec<'_>>, Error> {
        match (&self.dead_letter_queue, self.max_receive_count) {
            (None, None) => Ok(None),
            (Some(queue), Some(max_receive_count)) => Ok(Some(DeadLetterSpec {
                queue,
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
    /// A new queue's attributes: defaults overridden by `updates`.
    pub fn for_new_queue(updates: &AttributeUpdates, dead_letter: Option<DeadLetter>) -> Self {
        let kind = if updates.fifo.unwrap_or(false) {
            Kind::Fifo {
                content_based_deduplication: false,
            }
        } else {
            Kind::Standard
        };
        let mut attributes = Self {
            visibility_timeout: DEFAULT_VISIBILITY_TIMEOUT,
            delay_seconds: DEFAULT_DELAY_SECONDS,
            retention_seconds: DEFAULT_RETENTION_SECONDS,
            kind,
            dead_letter: None,
        };
        attributes.apply(updates, dead_letter);
        attributes
    }

    /// Takes the attributes `updates` names. `dead_letter` replaces the
    /// configuration when present and never removes it. `Fifo` is ignored:
    /// a queue's type does not change.
    pub fn apply(&mut self, updates: &AttributeUpdates, dead_letter: Option<DeadLetter>) {
        if let Some(value) = updates.visibility_timeout {
            self.visibility_timeout = value;
        }
        if let Some(value) = updates.delay_seconds {
            self.delay_seconds = value;
        }
        if let Some(value) = updates.retention_seconds {
            self.retention_seconds = value;
        }
        if let (
            Kind::Fifo {
                content_based_deduplication,
            },
            Some(value),
        ) = (&mut self.kind, updates.content_based_deduplication)
        {
            *content_based_deduplication = value;
        }
        if dead_letter.is_some() {
            self.dead_letter = dead_letter;
        }
    }

    /// Whether `CreateQueue` with `updates` and `spec` asks for exactly these
    /// attributes (an unset dead-letter configuration equals an unset one).
    pub fn matches_create(
        &self,
        updates: &AttributeUpdates,
        spec: Option<DeadLetterSpec<'_>>,
    ) -> bool {
        let wanted = Self::for_new_queue(updates, None);
        let same_dead_letter = match (&self.dead_letter, spec) {
            (None, None) => true,
            (Some(have), Some(want)) => {
                have.queue.as_str() == want.queue
                    && have.max_receive_count == want.max_receive_count
            }
            _ => false,
        };
        same_dead_letter
            && Self {
                dead_letter: None,
                ..self.clone()
            } == wanted
    }

    /// Whether this is a FIFO queue's attribute set.
    pub fn is_fifo(&self) -> bool {
        matches!(self.kind, Kind::Fifo { .. })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn updates(value: &Value) -> Result<AttributeUpdates, Error> {
        AttributeUpdates::parse(value.as_object().expect("literal is an object"))
    }

    #[test]
    fn unknown_and_ill_typed_attributes_are_invalid_values() {
        let cases = [
            json!({"Nope": 1}),
            json!({"VisibilityTimeout": 43201}),
            json!({"RetentionSeconds": 59}),
            json!({"Fifo": "true"}),
            json!({"MaxReceiveCount": 0}),
            json!({"DeadLetterQueue": 5}),
        ];
        for case in cases {
            let error = updates(&case).expect_err("attribute must be rejected");
            assert_eq!(error.code(), ErrorCode::InvalidParameterValue, "{case}");
        }
    }

    #[test]
    fn null_attributes_are_absent() {
        let parsed = updates(&json!({"VisibilityTimeout": null, "Fifo": null, "Nope": null}))
            .expect("nulls are ignored");
        assert_eq!(parsed.fifo(), None);
        assert!(parsed.dead_letter().expect("nothing set").is_none());
    }

    #[test]
    fn half_a_dead_letter_configuration_is_invalid() {
        let parsed = updates(&json!({"MaxReceiveCount": 3})).expect("each attribute is valid");
        assert_eq!(
            parsed.dead_letter().expect_err("only one of two").code(),
            ErrorCode::InvalidParameterValue
        );
    }

    #[test]
    fn new_queue_defaults_serialize_as_listed_in_the_specification() {
        let attributes = Attributes::for_new_queue(&AttributeUpdates::default(), None);
        assert_eq!(
            serde_json::to_value(&attributes).expect("serializable"),
            json!({
                "VisibilityTimeout": 30,
                "DelaySeconds": 0,
                "RetentionSeconds": 345_600,
                "Fifo": false,
                "ContentBasedDeduplication": false,
            })
        );
    }
}
