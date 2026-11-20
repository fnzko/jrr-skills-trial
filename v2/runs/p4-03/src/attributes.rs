//! Queue attributes (SPEC section 4). A request names some of them, so its fields are optional; a
//! stored queue has all of them, filled from the defaults.

use crate::error::Failure;
use crate::ids::QueueName;
use crate::params::{boolean, bounded, invalid, string};
use crate::time::Seconds;
use serde_json::Value;

/// The attributes a request names. None means the request left the attribute out, which is not
/// the same as naming its default.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AttributeUpdate {
    visibility_timeout: Option<Seconds>,
    delay_seconds: Option<Seconds>,
    retention_seconds: Option<Seconds>,
    fifo: Option<bool>,
    content_based_deduplication: Option<bool>,
    dead_letter_queue: Option<String>,
    max_receive_count: Option<u32>,
}

impl AttributeUpdate {
    /// Reads an `Attributes` member. A missing member names no attributes. Otherwise the value
    /// must be an object whose members are attributes of section 4 with values in range; a member
    /// whose value is null counts as absent.
    pub fn parse(value: Option<&Value>) -> Result<AttributeUpdate, Failure> {
        let Some(value) = value else {
            return Ok(AttributeUpdate::default());
        };
        let Value::Object(members) = value else {
            return Err(invalid("Attributes", "must be an object"));
        };
        let mut update = AttributeUpdate::default();
        for (name, value) in members {
            if value.is_null() {
                continue;
            }
            match name.as_str() {
                "VisibilityTimeout" => {
                    update.visibility_timeout = Some(Seconds::new(bounded(name, value, 0, 43_200)?))
                }
                "DelaySeconds" => {
                    update.delay_seconds = Some(Seconds::new(bounded(name, value, 0, 900)?))
                }
                "RetentionSeconds" => {
                    update.retention_seconds =
                        Some(Seconds::new(bounded(name, value, 60, 1_209_600)?))
                }
                "Fifo" => update.fifo = Some(boolean(name, value)?),
                "ContentBasedDeduplication" => {
                    update.content_based_deduplication = Some(boolean(name, value)?)
                }
                "DeadLetterQueue" => {
                    update.dead_letter_queue = Some(string(name, value)?.to_owned())
                }
                "MaxReceiveCount" => {
                    update.max_receive_count = Some(bounded(name, value, 1, 1000)?)
                }
                _ => return Err(invalid(name, "is not a queue attribute")),
            }
        }
        Ok(update)
    }

    /// The Fifo member, if the request names it, whatever its value.
    pub fn fifo(&self) -> Option<bool> {
        self.fifo
    }

    pub fn content_based_deduplication(&self) -> Option<bool> {
        self.content_based_deduplication
    }

    /// The dead-letter configuration the request names. Its two fields come together or not at
    /// all.
    pub fn dead_letter(&self) -> Result<Option<DeadLetterRequest<'_>>, Failure> {
        match (&self.dead_letter_queue, self.max_receive_count) {
            (None, None) => Ok(None),
            (Some(queue), Some(max_receive_count)) => Ok(Some(DeadLetterRequest {
                queue,
                max_receive_count,
            })),
            _ => Err(invalid(
                "DeadLetterQueue",
                "and MaxReceiveCount must be given together",
            )),
        }
    }
}

/// A dead-letter configuration as a request names it. The queue it names is not checked yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeadLetterRequest<'a> {
    queue: &'a str,
    max_receive_count: u32,
}

impl<'a> DeadLetterRequest<'a> {
    pub fn queue(&self) -> &'a str {
        self.queue
    }

    pub fn max_receive_count(&self) -> u32 {
        self.max_receive_count
    }
}

/// A stored dead-letter configuration. Both fields are here or neither is, and the queue exists:
/// deleting that queue clears the configuration (SPEC section 5, DeleteQueue).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeadLetter {
    queue: QueueName,
    max_receive_count: u32,
}

impl DeadLetter {
    /// Takes a request whose queue the caller has already found to exist.
    pub fn from_request(queue: QueueName, request: DeadLetterRequest<'_>) -> DeadLetter {
        DeadLetter {
            queue,
            max_receive_count: request.max_receive_count,
        }
    }

    pub fn queue(&self) -> &QueueName {
        &self.queue
    }

    pub fn max_receive_count(&self) -> u32 {
        self.max_receive_count
    }
}

/// The complete attributes of a queue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueConfig {
    fifo: bool,
    visibility_timeout: Seconds,
    delay_seconds: Seconds,
    retention_seconds: Seconds,
    content_based_deduplication: bool,
    dead_letter: Option<DeadLetter>,
}

impl QueueConfig {
    /// The defaults of section 4.
    pub fn defaults() -> QueueConfig {
        QueueConfig {
            fifo: false,
            visibility_timeout: Seconds::new(30),
            delay_seconds: Seconds::new(0),
            retention_seconds: Seconds::new(345_600),
            content_based_deduplication: false,
            dead_letter: None,
        }
    }

    /// This config with the attributes `update` names replaced. `dead_letter` replaces the
    /// dead-letter configuration when it is Some; otherwise the current one stays.
    pub fn updated(
        &self,
        update: &AttributeUpdate,
        dead_letter: Option<DeadLetter>,
    ) -> QueueConfig {
        QueueConfig {
            fifo: update.fifo.unwrap_or(self.fifo),
            visibility_timeout: update.visibility_timeout.unwrap_or(self.visibility_timeout),
            delay_seconds: update.delay_seconds.unwrap_or(self.delay_seconds),
            retention_seconds: update.retention_seconds.unwrap_or(self.retention_seconds),
            content_based_deduplication: update
                .content_based_deduplication
                .unwrap_or(self.content_based_deduplication),
            dead_letter: dead_letter.or_else(|| self.dead_letter.clone()),
        }
    }

    pub fn fifo(&self) -> bool {
        self.fifo
    }

    pub fn visibility_timeout(&self) -> Seconds {
        self.visibility_timeout
    }

    pub fn delay_seconds(&self) -> Seconds {
        self.delay_seconds
    }

    pub fn retention_seconds(&self) -> Seconds {
        self.retention_seconds
    }

    pub fn content_based_deduplication(&self) -> bool {
        self.content_based_deduplication
    }

    pub fn dead_letter(&self) -> Option<&DeadLetter> {
        self.dead_letter.as_ref()
    }

    /// This config with no dead-letter configuration, for a dead-letter queue that was deleted.
    pub fn without_dead_letter(&self) -> QueueConfig {
        QueueConfig {
            dead_letter: None,
            ..self.clone()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorCode;
    use serde_json::json;

    fn one(name: &str, value: Value) -> Value {
        Value::Object(std::iter::once((name.to_owned(), value)).collect())
    }

    #[test]
    fn each_attribute_accepts_its_range_and_rejects_values_outside_it() {
        let cases: Vec<(&str, Value, Result<(), ErrorCode>)> = vec![
            ("VisibilityTimeout", json!(0), Ok(())),
            ("VisibilityTimeout", json!(43_200), Ok(())),
            (
                "VisibilityTimeout",
                json!(43_201),
                Err(ErrorCode::InvalidParameterValue),
            ),
            (
                "VisibilityTimeout",
                json!(-1),
                Err(ErrorCode::InvalidParameterValue),
            ),
            (
                "VisibilityTimeout",
                json!(30.0),
                Err(ErrorCode::InvalidParameterValue),
            ),
            (
                "VisibilityTimeout",
                json!("30"),
                Err(ErrorCode::InvalidParameterValue),
            ),
            ("DelaySeconds", json!(900), Ok(())),
            (
                "DelaySeconds",
                json!(901),
                Err(ErrorCode::InvalidParameterValue),
            ),
            (
                "RetentionSeconds",
                json!(59),
                Err(ErrorCode::InvalidParameterValue),
            ),
            ("RetentionSeconds", json!(60), Ok(())),
            ("RetentionSeconds", json!(1_209_600), Ok(())),
            (
                "RetentionSeconds",
                json!(1_209_601),
                Err(ErrorCode::InvalidParameterValue),
            ),
            ("Fifo", json!(true), Ok(())),
            ("Fifo", json!("true"), Err(ErrorCode::InvalidParameterValue)),
            ("ContentBasedDeduplication", json!(false), Ok(())),
            (
                "ContentBasedDeduplication",
                json!(1),
                Err(ErrorCode::InvalidParameterValue),
            ),
            ("DeadLetterQueue", json!("dead"), Ok(())),
            (
                "DeadLetterQueue",
                json!(5),
                Err(ErrorCode::InvalidParameterValue),
            ),
            (
                "MaxReceiveCount",
                json!(0),
                Err(ErrorCode::InvalidParameterValue),
            ),
            ("MaxReceiveCount", json!(1), Ok(())),
            ("MaxReceiveCount", json!(1000), Ok(())),
            (
                "MaxReceiveCount",
                json!(1001),
                Err(ErrorCode::InvalidParameterValue),
            ),
            ("Colour", json!(1), Err(ErrorCode::InvalidParameterValue)),
        ];
        for (name, value, expected) in cases {
            let result = AttributeUpdate::parse(Some(&one(name, value.clone())))
                .map(|_| ())
                .map_err(|failure| failure.code());
            assert_eq!(result, expected, "{name} = {value}");
        }
    }

    #[test]
    fn attributes_must_be_an_object_and_null_members_are_absent() {
        assert_eq!(
            AttributeUpdate::parse(Some(&json!(["Fifo"])))
                .map(|_| ())
                .map_err(|failure| failure.code()),
            Err(ErrorCode::InvalidParameterValue)
        );
        assert_eq!(
            AttributeUpdate::parse(Some(&one("Fifo", Value::Null))),
            Ok(AttributeUpdate::default())
        );
        assert_eq!(AttributeUpdate::parse(None), Ok(AttributeUpdate::default()));
    }

    #[test]
    fn dead_letter_queue_and_max_receive_count_come_together() {
        let both = json!({"DeadLetterQueue": "dead", "MaxReceiveCount": 3});
        let only_queue = json!({"DeadLetterQueue": "dead"});
        let only_max = json!({"MaxReceiveCount": 3});
        let neither = json!({"Fifo": false});
        type Expected<'a> = Result<Option<(&'a str, u32)>, ErrorCode>;
        let cases: Vec<(Value, Expected)> = vec![
            (both, Ok(Some(("dead", 3)))),
            (only_queue, Err(ErrorCode::InvalidParameterValue)),
            (only_max, Err(ErrorCode::InvalidParameterValue)),
            (neither, Ok(None)),
        ];
        for (members, expected) in cases {
            let update = AttributeUpdate::parse(Some(&members)).unwrap();
            let result = update
                .dead_letter()
                .map(|request| request.map(|r| (r.queue(), r.max_receive_count())))
                .map_err(|failure| failure.code());
            assert_eq!(result, expected, "{members}");
        }
    }

    #[test]
    fn defaults_match_section_four() {
        assert_eq!(
            QueueConfig::defaults(),
            QueueConfig {
                fifo: false,
                visibility_timeout: Seconds::new(30),
                delay_seconds: Seconds::new(0),
                retention_seconds: Seconds::new(345_600),
                content_based_deduplication: false,
                dead_letter: None,
            }
        );
    }

    #[test]
    fn update_replaces_only_the_attributes_it_names() {
        let update = AttributeUpdate::parse(Some(&json!({"VisibilityTimeout": 5}))).unwrap();
        let config = QueueConfig::defaults().updated(&update, None);
        assert_eq!(
            config,
            QueueConfig {
                visibility_timeout: Seconds::new(5),
                ..QueueConfig::defaults()
            }
        );
    }
}
