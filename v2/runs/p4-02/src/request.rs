//! Parsing of command objects into typed requests.
//!
//! This module performs the checks of SPEC section 2 that need no queue state, in the order the
//! spec prescribes: request shape, then required parameters, then each parameter's value. Anything
//! that depends on existing queues or messages is left to [`crate::service`].

use std::ops::RangeInclusive;

use serde_json::{Map, Value};

use crate::error::{Error, ErrorCode, Result};

pub const VISIBILITY_TIMEOUT_RANGE: RangeInclusive<u64> = 0..=43_200;
pub const DELAY_SECONDS_RANGE: RangeInclusive<u64> = 0..=900;
pub const RETENTION_SECONDS_RANGE: RangeInclusive<u64> = 60..=1_209_600;
pub const MAX_RECEIVE_COUNT_RANGE: RangeInclusive<u64> = 1..=1000;
pub const MAX_MESSAGES_RANGE: RangeInclusive<u64> = 1..=10;
pub const TICK_SECONDS_RANGE: RangeInclusive<u64> = 0..=1_209_600;
pub const BODY_BYTES_RANGE: RangeInclusive<usize> = 1..=1024;
pub const MESSAGE_TOKEN_LEN_RANGE: RangeInclusive<usize> = 1..=128;

/// A queue attribute update as given in `CreateQueue` or `SetQueueAttributes`.
///
/// Every field is `None` when the attribute was absent (or `null`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AttributeChanges {
    pub visibility_timeout: Option<u64>,
    pub delay_seconds: Option<u64>,
    pub retention_seconds: Option<u64>,
    pub fifo: Option<bool>,
    pub content_based_deduplication: Option<bool>,
    pub dead_letter_queue: Option<String>,
    pub max_receive_count: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendMessage {
    pub queue_name: String,
    pub body: String,
    pub delay_seconds: Option<u64>,
    pub group_id: Option<String>,
    pub deduplication_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceiveMessage {
    pub queue_name: String,
    pub max_messages: u64,
    pub visibility_timeout: Option<u64>,
}

/// A command whose parameters have passed every state-independent check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    CreateQueue {
        queue_name: String,
        attributes: AttributeChanges,
    },
    SetQueueAttributes {
        queue_name: String,
        attributes: AttributeChanges,
    },
    GetQueueAttributes {
        queue_name: String,
    },
    ListQueues {
        prefix: Option<String>,
    },
    DeleteQueue {
        queue_name: String,
    },
    PurgeQueue {
        queue_name: String,
    },
    Tick {
        seconds: u64,
    },
    SendMessage(SendMessage),
    ReceiveMessage(ReceiveMessage),
    DeleteMessage {
        queue_name: String,
        receipt_handle: String,
    },
    ChangeMessageVisibility {
        queue_name: String,
        receipt_handle: String,
        visibility_timeout: u64,
    },
}

impl Request {
    /// Parses one input line. Fails with `InvalidRequest` unless the line is a JSON object with a
    /// string `Command` naming a known command.
    pub fn from_line(line: &[u8]) -> Result<Self> {
        let text = std::str::from_utf8(line)
            .map_err(|_| Error::invalid_request("line is not valid UTF-8"))?;
        let value: Value = serde_json::from_str(text)
            .map_err(|e| Error::invalid_request(format!("line is not valid JSON: {e}")))?;
        let Value::Object(object) = value else {
            return Err(Error::invalid_request("line is not a JSON object"));
        };
        Self::from_object(&object)
    }

    /// Parses a command object; see [`Request::from_line`].
    pub fn from_object(object: &Map<String, Value>) -> Result<Self> {
        let params = Params(object);
        let Some(Value::String(command)) = params.get("Command") else {
            return Err(Error::invalid_request("missing string member Command"));
        };
        match command.as_str() {
            "CreateQueue" => {
                params.require(&["QueueName"])?;
                // CreateQueue checks QueueName before anything else, with its own error code.
                let queue_name = params.string("QueueName")?;
                validate_queue_name(&queue_name)?;
                Ok(Self::CreateQueue {
                    queue_name,
                    attributes: params.attributes()?.unwrap_or_default(),
                })
            }
            "SetQueueAttributes" => {
                params.require(&["QueueName", "Attributes"])?;
                Ok(Self::SetQueueAttributes {
                    queue_name: params.string("QueueName")?,
                    attributes: params.attributes()?.unwrap_or_default(),
                })
            }
            "GetQueueAttributes" => Ok(Self::GetQueueAttributes {
                queue_name: params.queue_name()?,
            }),
            "ListQueues" => Ok(Self::ListQueues {
                prefix: params.optional_string("Prefix")?,
            }),
            "DeleteQueue" => Ok(Self::DeleteQueue {
                queue_name: params.queue_name()?,
            }),
            "PurgeQueue" => Ok(Self::PurgeQueue {
                queue_name: params.queue_name()?,
            }),
            "Tick" => {
                params.require(&["Seconds"])?;
                Ok(Self::Tick {
                    seconds: params.integer("Seconds", TICK_SECONDS_RANGE)?,
                })
            }
            "SendMessage" => {
                params.require(&["QueueName", "Body"])?;
                let queue_name = params.string("QueueName")?;
                let body = params.string("Body")?;
                if !BODY_BYTES_RANGE.contains(&body.len()) {
                    return Err(Error::invalid_value("Body must be 1 to 1024 bytes long"));
                }
                Ok(Self::SendMessage(SendMessage {
                    queue_name,
                    body,
                    delay_seconds: params.optional_integer("DelaySeconds", DELAY_SECONDS_RANGE)?,
                    group_id: params.optional_token("GroupId")?,
                    deduplication_id: params.optional_token("DeduplicationId")?,
                }))
            }
            "ReceiveMessage" => {
                params.require(&["QueueName"])?;
                Ok(Self::ReceiveMessage(ReceiveMessage {
                    queue_name: params.string("QueueName")?,
                    max_messages: params
                        .optional_integer("MaxMessages", MAX_MESSAGES_RANGE)?
                        .unwrap_or(1),
                    visibility_timeout: params
                        .optional_integer("VisibilityTimeout", VISIBILITY_TIMEOUT_RANGE)?,
                }))
            }
            "DeleteMessage" => {
                params.require(&["QueueName", "ReceiptHandle"])?;
                Ok(Self::DeleteMessage {
                    queue_name: params.string("QueueName")?,
                    receipt_handle: params.string("ReceiptHandle")?,
                })
            }
            "ChangeMessageVisibility" => {
                params.require(&["QueueName", "ReceiptHandle", "VisibilityTimeout"])?;
                Ok(Self::ChangeMessageVisibility {
                    queue_name: params.string("QueueName")?,
                    receipt_handle: params.string("ReceiptHandle")?,
                    visibility_timeout: params
                        .integer("VisibilityTimeout", VISIBILITY_TIMEOUT_RANGE)?,
                })
            }
            other => Err(Error::invalid_request(format!("unknown command {other:?}"))),
        }
    }
}

/// Checks the queue name syntax of SPEC section 4.
pub fn validate_queue_name(name: &str) -> Result<()> {
    let stem = name.strip_suffix(".fifo").unwrap_or(name);
    let stem_limit = if stem.len() == name.len() { 80 } else { 75 };
    let valid = (1..=stem_limit).contains(&stem.len()) && stem.bytes().all(is_name_byte);
    if valid {
        Ok(())
    } else {
        Err(Error::new(
            ErrorCode::QueueNameInvalid,
            format!("{name:?} is not a valid queue name"),
        ))
    }
}

fn is_name_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-'
}

/// Read access to a command object's members, treating `null` as absent.
struct Params<'a>(&'a Map<String, Value>);

impl<'a> Params<'a> {
    fn get(&self, name: &str) -> Option<&'a Value> {
        self.0.get(name).filter(|value| !value.is_null())
    }

    /// Fails with `MissingParameter` on the first absent name. Called before any value check so
    /// that a missing parameter wins over an invalid one, as the spec orders them.
    fn require(&self, names: &[&str]) -> Result<()> {
        match names.iter().find(|name| self.get(name).is_none()) {
            Some(name) => Err(Error::missing(name)),
            None => Ok(()),
        }
    }

    fn queue_name(&self) -> Result<String> {
        self.require(&["QueueName"])?;
        self.string("QueueName")
    }

    /// A parameter already known to be present (see [`Params::require`]).
    fn present(&self, name: &str) -> Result<&'a Value> {
        self.get(name).ok_or_else(|| Error::missing(name))
    }

    fn string(&self, name: &str) -> Result<String> {
        expect_string(self.present(name)?, name)
    }

    fn optional_string(&self, name: &str) -> Result<Option<String>> {
        self.get(name).map(|v| expect_string(v, name)).transpose()
    }

    fn integer(&self, name: &str, range: RangeInclusive<u64>) -> Result<u64> {
        expect_integer(self.present(name)?, name, range)
    }

    fn optional_integer(&self, name: &str, range: RangeInclusive<u64>) -> Result<Option<u64>> {
        self.get(name)
            .map(|v| expect_integer(v, name, range))
            .transpose()
    }

    /// A `GroupId` or `DeduplicationId`: 1 to 128 characters from `A-Z a-z 0-9 _ -`.
    fn optional_token(&self, name: &str) -> Result<Option<String>> {
        let Some(token) = self.optional_string(name)? else {
            return Ok(None);
        };
        if MESSAGE_TOKEN_LEN_RANGE.contains(&token.len()) && token.bytes().all(is_name_byte) {
            Ok(Some(token))
        } else {
            Err(Error::invalid_value(format!(
                "{name} must be 1 to 128 characters from A-Z a-z 0-9 _ -"
            )))
        }
    }

    fn attributes(&self) -> Result<Option<AttributeChanges>> {
        let Some(value) = self.get("Attributes") else {
            return Ok(None);
        };
        let Value::Object(object) = value else {
            return Err(Error::invalid_value("Attributes must be an object"));
        };
        let attrs = Params(object);
        // Reject unknown names before looking at values so the outcome does not depend on the
        // member order of the input.
        if let Some(unknown) = object
            .iter()
            .filter(|(_, value)| !value.is_null())
            .map(|(name, _)| name)
            .find(|name| !ATTRIBUTE_NAMES.contains(&name.as_str()))
        {
            return Err(Error::invalid_value(format!(
                "unknown attribute {unknown:?}"
            )));
        }
        Ok(Some(AttributeChanges {
            visibility_timeout: attrs
                .optional_integer("VisibilityTimeout", VISIBILITY_TIMEOUT_RANGE)?,
            delay_seconds: attrs.optional_integer("DelaySeconds", DELAY_SECONDS_RANGE)?,
            retention_seconds: attrs
                .optional_integer("RetentionSeconds", RETENTION_SECONDS_RANGE)?,
            fifo: attrs.optional_boolean("Fifo")?,
            content_based_deduplication: attrs.optional_boolean("ContentBasedDeduplication")?,
            dead_letter_queue: attrs.optional_string("DeadLetterQueue")?,
            max_receive_count: attrs
                .optional_integer("MaxReceiveCount", MAX_RECEIVE_COUNT_RANGE)?,
        }))
    }

    fn optional_boolean(&self, name: &str) -> Result<Option<bool>> {
        self.get(name)
            .map(|value| {
                value
                    .as_bool()
                    .ok_or_else(|| Error::invalid_value(format!("{name} must be a boolean")))
            })
            .transpose()
    }
}

const ATTRIBUTE_NAMES: [&str; 7] = [
    "VisibilityTimeout",
    "DelaySeconds",
    "RetentionSeconds",
    "Fifo",
    "ContentBasedDeduplication",
    "DeadLetterQueue",
    "MaxReceiveCount",
];

fn expect_string(value: &Value, name: &str) -> Result<String> {
    value
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| Error::invalid_value(format!("{name} must be a string")))
}

/// Accepts only JSON integers: serde_json parses any number written with a fraction or exponent
/// as a float, for which `as_u64` returns `None`.
fn expect_integer(value: &Value, name: &str, range: RangeInclusive<u64>) -> Result<u64> {
    value.as_u64().filter(|n| range.contains(n)).ok_or_else(|| {
        Error::invalid_value(format!(
            "{name} must be an integer from {} to {}",
            range.start(),
            range.end()
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(line: &str) -> Result<Request> {
        Request::from_line(line.as_bytes())
    }

    fn code(line: &str) -> ErrorCode {
        parse(line).expect_err("expected an error").code
    }

    #[test]
    fn rejects_malformed_lines() {
        assert_eq!(code(""), ErrorCode::InvalidRequest);
        assert_eq!(code("[1]"), ErrorCode::InvalidRequest);
        assert_eq!(code("{\"Command\": 3}"), ErrorCode::InvalidRequest);
        assert_eq!(code("{\"Command\": null}"), ErrorCode::InvalidRequest);
        assert_eq!(code("{\"Command\": \"Nope\"}"), ErrorCode::InvalidRequest);
        assert_eq!(code("{\"command\": \"Tick\"}"), ErrorCode::InvalidRequest);
        assert_eq!(code("{} {}"), ErrorCode::InvalidRequest);
        assert_eq!(
            Request::from_line(b"{\"Command\": \"\xff\"}")
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }

    #[test]
    fn integers_must_be_written_as_integers() {
        assert_eq!(
            parse(r#"{"Command": "Tick", "Seconds": 30}"#),
            Ok(Request::Tick { seconds: 30 })
        );
        for bad in ["30.0", "3e1", "\"30\"", "-1", "1209601", "true"] {
            let line = format!(r#"{{"Command": "Tick", "Seconds": {bad}}}"#);
            assert_eq!(code(&line), ErrorCode::InvalidParameterValue, "{bad}");
        }
    }

    #[test]
    fn null_counts_as_absent() {
        assert_eq!(
            code(r#"{"Command": "Tick", "Seconds": null}"#),
            ErrorCode::MissingParameter
        );
        assert_eq!(
            parse(r#"{"Command": "ListQueues", "Prefix": null, "Extra": 1}"#),
            Ok(Request::ListQueues { prefix: None })
        );
    }

    #[test]
    fn missing_parameter_beats_invalid_value() {
        assert_eq!(
            code(r#"{"Command": "SendMessage", "QueueName": 1}"#),
            ErrorCode::MissingParameter
        );
        assert_eq!(
            code(r#"{"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": null}"#),
            ErrorCode::MissingParameter
        );
    }

    #[test]
    fn create_queue_checks_name_first() {
        assert_eq!(
            code(r#"{"Command": "CreateQueue", "QueueName": "a b", "Attributes": 1}"#),
            ErrorCode::QueueNameInvalid
        );
        assert_eq!(
            code(r#"{"Command": "CreateQueue", "QueueName": 7, "Attributes": 1}"#),
            ErrorCode::InvalidParameterValue
        );
        assert_eq!(
            code(r#"{"Command": "CreateQueue", "QueueName": "ok", "Attributes": 1}"#),
            ErrorCode::InvalidParameterValue
        );
    }

    #[test]
    fn queue_name_syntax() {
        let ok = |name: &str| validate_queue_name(name).is_ok();
        assert!(ok("a"));
        assert!(ok("Az09_-"));
        assert!(ok(&"x".repeat(80)));
        assert!(ok(&format!("{}.fifo", "x".repeat(75))));
        assert!(ok("x.fifo"));
        assert!(!ok(""));
        assert!(!ok(".fifo"));
        assert!(!ok(&"x".repeat(81)));
        assert!(!ok(&format!("{}.fifo", "x".repeat(76))));
        assert!(!ok("a.b"));
        assert!(!ok("a.fifo.fifo"));
        assert!(!ok("é"));
    }

    #[test]
    fn attributes_are_validated() {
        let create = |attrs: &str| {
            parse(&format!(
                r#"{{"Command": "CreateQueue", "QueueName": "q", "Attributes": {attrs}}}"#
            ))
        };
        assert_eq!(
            create(r#"{"VisibilityTimeout": 0, "Fifo": null, "MaxReceiveCount": 5}"#),
            Ok(Request::CreateQueue {
                queue_name: "q".into(),
                attributes: AttributeChanges {
                    visibility_timeout: Some(0),
                    max_receive_count: Some(5),
                    ..Default::default()
                },
            })
        );
        for bad in [
            r#"{"Colour": 1}"#,
            r#"{"RetentionSeconds": 59}"#,
            r#"{"DelaySeconds": 901}"#,
            r#"{"MaxReceiveCount": 0}"#,
            r#"{"Fifo": "true"}"#,
            r#"{"DeadLetterQueue": 1}"#,
        ] {
            assert_eq!(
                create(bad).unwrap_err().code,
                ErrorCode::InvalidParameterValue,
                "{bad}"
            );
        }
    }

    #[test]
    fn send_message_values() {
        let send = |extra: &str| {
            parse(&format!(
                r#"{{"Command": "SendMessage", "QueueName": "q", "Body": "b"{extra}}}"#
            ))
        };
        assert!(send("").is_ok());
        assert!(send(&format!(r#", "GroupId": "{}""#, "g".repeat(128))).is_ok());
        let body = |b: &str| {
            parse(&format!(
                r#"{{"Command": "SendMessage", "QueueName": "q", "Body": "{b}"}}"#
            ))
        };
        assert!(body(&"é".repeat(512)).is_ok());
        assert_eq!(
            body(&"é".repeat(513)).unwrap_err().code,
            ErrorCode::InvalidParameterValue
        );
        assert_eq!(body("").unwrap_err().code, ErrorCode::InvalidParameterValue);
        for bad in [
            r#", "GroupId": """#,
            r#", "GroupId": "a.b""#,
            r#", "DeduplicationId": 5"#,
            r#", "DelaySeconds": 901"#,
        ] {
            assert_eq!(
                send(bad).unwrap_err().code,
                ErrorCode::InvalidParameterValue,
                "{bad}"
            );
        }
        assert_eq!(
            send(&format!(r#", "GroupId": "{}""#, "g".repeat(129)))
                .unwrap_err()
                .code,
            ErrorCode::InvalidParameterValue
        );
    }
}
