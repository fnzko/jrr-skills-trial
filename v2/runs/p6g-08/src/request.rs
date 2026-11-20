//! Turns one input line into a typed [`Command`], applying the checks of
//! error-precedence steps 1 to 3: shape, required parameters, parameter values.

use serde_json::{Map, Value};

use crate::error::{Error, ErrorCode};
use crate::types::{
    Bounded, DeduplicationId, DelaySeconds, GroupId, MaxMessages, MaxReceiveCount, QueueName,
    RetentionSeconds, TickSeconds, VisibilityTimeout,
};

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    CreateQueue {
        name: QueueName,
        attributes: AttributeSet,
    },
    SetQueueAttributes {
        queue: String,
        attributes: AttributeSet,
    },
    GetQueueAttributes {
        queue: String,
    },
    ListQueues {
        prefix: Option<String>,
    },
    DeleteQueue {
        queue: String,
    },
    PurgeQueue {
        queue: String,
    },
    Tick {
        seconds: TickSeconds,
    },
    SendMessage {
        queue: String,
        message: NewMessage,
    },
    ReceiveMessage {
        queue: String,
        max_messages: Option<MaxMessages>,
        visibility_timeout: Option<VisibilityTimeout>,
    },
    DeleteMessage {
        queue: String,
        receipt_handle: String,
    },
    ChangeMessageVisibility {
        queue: String,
        receipt_handle: String,
        visibility_timeout: VisibilityTimeout,
    },
}

/// The attributes a request names. Cross-attribute rules belong to the
/// commands, so they are not checked here.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct AttributeSet {
    pub visibility_timeout: Option<VisibilityTimeout>,
    pub delay_seconds: Option<DelaySeconds>,
    pub retention_seconds: Option<RetentionSeconds>,
    pub fifo: Option<bool>,
    pub content_based_deduplication: Option<bool>,
    pub dead_letter_queue: Option<String>,
    pub max_receive_count: Option<MaxReceiveCount>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct NewMessage {
    pub body: String,
    pub delay_seconds: Option<DelaySeconds>,
    pub group_id: Option<GroupId>,
    pub deduplication_id: Option<DeduplicationId>,
}

pub fn parse(line: &[u8]) -> Result<Command, Error> {
    let invalid = |detail: &dyn std::fmt::Display| Error::new(ErrorCode::InvalidRequest, detail);
    let text = std::str::from_utf8(line).map_err(|e| invalid(&format!("not valid UTF-8: {e}")))?;
    let Value::Object(members) =
        serde_json::from_str(text).map_err(|e| invalid(&format!("not a JSON value: {e}")))?
    else {
        return Err(invalid(&"the line is not a JSON object"));
    };
    let Some(Value::String(name)) = members.get("Command") else {
        return Err(invalid(&"Command must be a string"));
    };
    let params = Params(&members);
    match name.as_str() {
        "CreateQueue" => {
            params.require(&["QueueName"])?;
            // The name is checked before every other parameter.
            let name = QueueName::new(&params.string("QueueName")?)?;
            Ok(Command::CreateQueue {
                name,
                attributes: params.attributes("Attributes")?.unwrap_or_default(),
            })
        }
        "SetQueueAttributes" => {
            params.require(&["QueueName", "Attributes"])?;
            Ok(Command::SetQueueAttributes {
                queue: params.string("QueueName")?,
                attributes: params
                    .attributes("Attributes")?
                    .expect("Attributes was required and is present"),
            })
        }
        "GetQueueAttributes" => Ok(Command::GetQueueAttributes {
            queue: params.queue()?,
        }),
        "ListQueues" => Ok(Command::ListQueues {
            prefix: params.optional("Prefix")?,
        }),
        "DeleteQueue" => Ok(Command::DeleteQueue {
            queue: params.queue()?,
        }),
        "PurgeQueue" => Ok(Command::PurgeQueue {
            queue: params.queue()?,
        }),
        "Tick" => {
            params.require(&["Seconds"])?;
            Ok(Command::Tick {
                seconds: params.value("Seconds")?,
            })
        }
        "SendMessage" => {
            params.require(&["QueueName", "Body"])?;
            Ok(Command::SendMessage {
                queue: params.string("QueueName")?,
                message: NewMessage {
                    body: params.body()?,
                    delay_seconds: params.optional("DelaySeconds")?,
                    group_id: params.optional("GroupId")?,
                    deduplication_id: params.optional("DeduplicationId")?,
                },
            })
        }
        "ReceiveMessage" => {
            params.require(&["QueueName"])?;
            Ok(Command::ReceiveMessage {
                queue: params.string("QueueName")?,
                max_messages: params.optional("MaxMessages")?,
                visibility_timeout: params.optional("VisibilityTimeout")?,
            })
        }
        "DeleteMessage" => {
            params.require(&["QueueName", "ReceiptHandle"])?;
            Ok(Command::DeleteMessage {
                queue: params.string("QueueName")?,
                receipt_handle: params.string("ReceiptHandle")?,
            })
        }
        "ChangeMessageVisibility" => {
            params.require(&["QueueName", "ReceiptHandle", "VisibilityTimeout"])?;
            Ok(Command::ChangeMessageVisibility {
                queue: params.string("QueueName")?,
                receipt_handle: params.string("ReceiptHandle")?,
                visibility_timeout: params.value("VisibilityTimeout")?,
            })
        }
        other => Err(invalid(&format!("unknown command {other:?}"))),
    }
}

/// Members of one command line. A `null` member counts as absent.
struct Params<'a>(&'a Map<String, Value>);

impl Params<'_> {
    fn get(&self, name: &str) -> Option<&Value> {
        self.0.get(name).filter(|value| !value.is_null())
    }

    /// Step 2 of the error precedence: runs before any value is looked at.
    fn require(&self, names: &[&str]) -> Result<(), Error> {
        match names.iter().find(|name| self.get(name).is_none()) {
            Some(name) => Err(Error::new(
                ErrorCode::MissingParameter,
                format!("{name} is required"),
            )),
            None => Ok(()),
        }
    }

    /// A parameter that `require` has already vouched for.
    fn value<T: FromValue>(&self, name: &str) -> Result<T, Error> {
        let value = self
            .get(name)
            .expect("callers name only parameters that require() checked");
        T::from_value(value, name)
    }

    fn optional<T: FromValue>(&self, name: &str) -> Result<Option<T>, Error> {
        self.get(name).map(|v| T::from_value(v, name)).transpose()
    }

    fn string(&self, name: &str) -> Result<String, Error> {
        self.value(name)
    }

    fn queue(&self) -> Result<String, Error> {
        self.require(&["QueueName"])?;
        self.string("QueueName")
    }

    fn body(&self) -> Result<String, Error> {
        let body: String = self.value("Body")?;
        if (1..=1024).contains(&body.len()) {
            Ok(body)
        } else {
            Err(invalid_value("Body must be 1 to 1024 bytes of UTF-8"))
        }
    }

    fn attributes(&self, name: &str) -> Result<Option<AttributeSet>, Error> {
        self.get(name)
            .map(|value| match value {
                Value::Object(members) => AttributeSet::from_members(members),
                _ => Err(invalid_value(format!("{name} must be an object"))),
            })
            .transpose()
    }
}

fn invalid_value(message: impl std::fmt::Display) -> Error {
    Error::new(ErrorCode::InvalidParameterValue, message)
}

impl AttributeSet {
    fn from_members(members: &Map<String, Value>) -> Result<Self, Error> {
        let mut set = Self::default();
        for (name, value) in members.iter().filter(|(_, value)| !value.is_null()) {
            match name.as_str() {
                "VisibilityTimeout" => {
                    set.visibility_timeout = Some(FromValue::from_value(value, name)?)
                }
                "DelaySeconds" => set.delay_seconds = Some(FromValue::from_value(value, name)?),
                "RetentionSeconds" => {
                    set.retention_seconds = Some(FromValue::from_value(value, name)?)
                }
                "Fifo" => set.fifo = Some(FromValue::from_value(value, name)?),
                "ContentBasedDeduplication" => {
                    set.content_based_deduplication = Some(FromValue::from_value(value, name)?);
                }
                "DeadLetterQueue" => {
                    set.dead_letter_queue = Some(FromValue::from_value(value, name)?)
                }
                "MaxReceiveCount" => {
                    set.max_receive_count = Some(FromValue::from_value(value, name)?)
                }
                other => return Err(invalid_value(format!("unknown attribute {other}"))),
            }
        }
        Ok(set)
    }
}

/// A JSON value of the right type and within the stated range or syntax.
trait FromValue: Sized {
    fn from_value(value: &Value, name: &str) -> Result<Self, Error>;
}

impl FromValue for String {
    fn from_value(value: &Value, name: &str) -> Result<Self, Error> {
        match value {
            Value::String(s) => Ok(s.clone()),
            _ => Err(invalid_value(format!("{name} must be a string"))),
        }
    }
}

impl FromValue for bool {
    fn from_value(value: &Value, name: &str) -> Result<Self, Error> {
        value
            .as_bool()
            .ok_or_else(|| invalid_value(format!("{name} must be true or false")))
    }
}

impl<const MIN: u64, const MAX: u64> FromValue for Bounded<MIN, MAX> {
    fn from_value(value: &Value, name: &str) -> Result<Self, Error> {
        // Only numbers written as plain integers convert to u64; negative
        // numbers lie below every range in the specification.
        value
            .as_u64()
            .and_then(Self::new)
            .ok_or_else(|| invalid_value(format!("{name} must be an integer from {MIN} to {MAX}")))
    }
}

impl FromValue for GroupId {
    fn from_value(value: &Value, name: &str) -> Result<Self, Error> {
        token(value, name, Self::new)
    }
}

impl FromValue for DeduplicationId {
    fn from_value(value: &Value, name: &str) -> Result<Self, Error> {
        token(value, name, Self::new)
    }
}

fn token<T>(value: &Value, name: &str, build: fn(&str) -> Option<T>) -> Result<T, Error> {
    let text: String = FromValue::from_value(value, name)?;
    build(&text).ok_or_else(|| {
        invalid_value(format!(
            "{name} must be 1 to 128 characters of A-Z a-z 0-9 _ -"
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code(line: &str) -> ErrorCode {
        parse(line.as_bytes()).unwrap_err().code
    }

    #[test]
    fn shape_errors_are_invalid_request() {
        let cases = [
            "",
            "   ",
            "[]",
            "\"x\"",
            "{",
            "{} trailing",
            "{\"Command\": 5}",
            "{\"Command\": null}",
            "{}",
            "{\"Command\": \"Nope\"}",
            "{\"command\": \"Tick\", \"Seconds\": 1}",
        ];
        for line in cases {
            assert_eq!(code(line), ErrorCode::InvalidRequest, "{line}");
        }
        assert_eq!(
            parse(b"{\"Command\":\"Tick\xff\"}").unwrap_err().code,
            ErrorCode::InvalidRequest
        );
    }

    #[test]
    fn missing_parameters_win_over_bad_values() {
        let line = r#"{"Command": "SendMessage", "QueueName": 5}"#;
        assert_eq!(code(line), ErrorCode::MissingParameter);
        let line = r#"{"Command": "SendMessage", "QueueName": "q", "Body": null}"#;
        assert_eq!(code(line), ErrorCode::MissingParameter);
    }

    #[test]
    fn integers_must_be_written_without_fraction_or_exponent() {
        for value in [
            "30.0",
            "3e1",
            "\"30\"",
            "-1",
            "43201",
            "true",
            "1e400",
            "99999999999999999999",
        ] {
            let line = format!(
                r#"{{"Command": "ReceiveMessage", "QueueName": "q", "VisibilityTimeout": {value}}}"#
            );
            assert_eq!(code(&line), ErrorCode::InvalidParameterValue, "{value}");
        }
        let line = r#"{"Command": "ReceiveMessage", "QueueName": "q", "VisibilityTimeout": 43200}"#;
        assert!(parse(line.as_bytes()).is_ok());
    }

    #[test]
    fn null_and_unknown_members_are_ignored() {
        let line =
            r#"{"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": null, "Extra": 1}"#;
        assert_eq!(
            parse(line.as_bytes()),
            Ok(Command::ReceiveMessage {
                queue: "q".to_owned(),
                max_messages: None,
                visibility_timeout: None,
            })
        );
    }

    #[test]
    fn create_queue_checks_the_name_before_other_parameters() {
        let line = r#"{"Command": "CreateQueue", "QueueName": "bad name", "Attributes": 5}"#;
        assert_eq!(code(line), ErrorCode::QueueNameInvalid);
        let line = r#"{"Command": "CreateQueue", "QueueName": "q", "Attributes": 5}"#;
        assert_eq!(code(line), ErrorCode::InvalidParameterValue);
    }

    #[test]
    fn attributes_reject_unknown_names_and_bad_values() {
        for attributes in [
            r#"{"Nope": 1}"#,
            r#"{"Fifo": "yes"}"#,
            r#"{"DelaySeconds": 901}"#,
            r#"{"MaxReceiveCount": 0}"#,
            "[]",
        ] {
            let line = format!(
                r#"{{"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {attributes}}}"#
            );
            assert_eq!(
                code(&line),
                ErrorCode::InvalidParameterValue,
                "{attributes}"
            );
        }
    }

    #[test]
    fn null_attributes_count_as_absent() {
        let line = r#"{"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"DelaySeconds": null}}"#;
        assert_eq!(
            parse(line.as_bytes()),
            Ok(Command::SetQueueAttributes {
                queue: "q".to_owned(),
                attributes: AttributeSet::default(),
            })
        );
        let line = r#"{"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": null}"#;
        assert_eq!(code(line), ErrorCode::MissingParameter);
    }

    #[test]
    fn body_length_is_counted_in_bytes() {
        let send = |body: &str| {
            format!(r#"{{"Command": "SendMessage", "QueueName": "q", "Body": "{body}"}}"#)
        };
        assert!(parse(send(&"a".repeat(1024)).as_bytes()).is_ok());
        assert_eq!(
            code(&send(&"a".repeat(1025))),
            ErrorCode::InvalidParameterValue
        );
        assert_eq!(code(&send("")), ErrorCode::InvalidParameterValue);
        // 513 two-byte characters are 1026 bytes.
        assert_eq!(
            code(&send(&"é".repeat(513))),
            ErrorCode::InvalidParameterValue
        );
        assert!(parse(send(&"é".repeat(512)).as_bytes()).is_ok());
    }
}
