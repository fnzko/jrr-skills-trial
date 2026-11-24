//! Turns one input line into a validated [`Command`].
//!
//! Checks run in the order the spec fixes: request shape, required
//! parameters, then parameter values. Rules that need queue state live in
//! the service.

use std::ops::RangeInclusive;

use serde_json::{Map, Value};

use crate::error::{Error, ErrorCode};
use crate::ids::{DedupKey, GroupId, QueueName};

/// Queue attributes as given on the wire; every member is optional.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AttributeInput {
    pub visibility_timeout: Option<u32>,
    pub delay_seconds: Option<u32>,
    pub retention_seconds: Option<u32>,
    pub fifo: Option<bool>,
    pub content_based_deduplication: Option<bool>,
    pub dead_letter_queue: Option<String>,
    pub max_receive_count: Option<u32>,
}

/// A parsed command. Queue and handle strings are unvalidated wire text:
/// lookups accept any string and fail with the code the spec names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    CreateQueue {
        name: QueueName,
        attributes: AttributeInput,
    },
    SetQueueAttributes {
        queue: String,
        attributes: AttributeInput,
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
        seconds: u32,
    },
    SendMessage {
        queue: String,
        body: String,
        delay_seconds: Option<u32>,
        group: Option<GroupId>,
        deduplication: Option<DedupKey>,
    },
    ReceiveMessage {
        queue: String,
        max_messages: u32,
        visibility_timeout: Option<u32>,
    },
    DeleteMessage {
        queue: String,
        receipt_handle: String,
    },
    ChangeMessageVisibility {
        queue: String,
        receipt_handle: String,
        visibility_timeout: u32,
    },
}

/// The members of the command object, with `null` read as absent.
struct Params<'a>(&'a Map<String, Value>);

impl<'a> Params<'a> {
    fn get(&self, name: &str) -> Option<&'a Value> {
        self.0.get(name).filter(|value| !value.is_null())
    }

    fn require(&self, name: &str) -> Result<&'a Value, Error> {
        self.get(name)
            .ok_or_else(|| Error::new(ErrorCode::MissingParameter, format!("{name} is required")))
    }
}

fn invalid(message: String) -> Error {
    Error::new(ErrorCode::InvalidParameterValue, message)
}

fn string<'a>(name: &str, value: &'a Value) -> Result<&'a str, Error> {
    value
        .as_str()
        .ok_or_else(|| invalid(format!("{name} must be a string")))
}

fn optional_string<'a>(params: &Params<'a>, name: &str) -> Result<Option<&'a str>, Error> {
    params
        .get(name)
        .map(|value| string(name, value))
        .transpose()
}

fn integer(name: &str, value: &Value, range: RangeInclusive<u32>) -> Result<u32, Error> {
    // `as_i64` is `None` for numbers written with a fraction or exponent.
    value
        .as_i64()
        .and_then(|number| u32::try_from(number).ok())
        .filter(|number| range.contains(number))
        .ok_or_else(|| {
            invalid(format!(
                "{name} must be an integer from {} to {}",
                range.start(),
                range.end()
            ))
        })
}

fn optional_integer(
    params: &Params<'_>,
    name: &str,
    range: RangeInclusive<u32>,
) -> Result<Option<u32>, Error> {
    params
        .get(name)
        .map(|value| integer(name, value, range))
        .transpose()
}

fn boolean(name: &str, value: &Value) -> Result<bool, Error> {
    value
        .as_bool()
        .ok_or_else(|| invalid(format!("{name} must be a boolean")))
}

fn attributes(value: Option<&Value>) -> Result<AttributeInput, Error> {
    let Some(value) = value else {
        return Ok(AttributeInput::default());
    };
    let members = value
        .as_object()
        .ok_or_else(|| invalid("Attributes must be an object".to_owned()))?;
    let mut input = AttributeInput::default();
    for (name, value) in members.iter().filter(|(_, value)| !value.is_null()) {
        match name.as_str() {
            "VisibilityTimeout" => {
                input.visibility_timeout = Some(integer(name, value, 0..=43_200)?);
            }
            "DelaySeconds" => input.delay_seconds = Some(integer(name, value, 0..=900)?),
            "RetentionSeconds" => {
                input.retention_seconds = Some(integer(name, value, 60..=1_209_600)?);
            }
            "Fifo" => input.fifo = Some(boolean(name, value)?),
            "ContentBasedDeduplication" => {
                input.content_based_deduplication = Some(boolean(name, value)?);
            }
            "DeadLetterQueue" => input.dead_letter_queue = Some(string(name, value)?.to_owned()),
            "MaxReceiveCount" => input.max_receive_count = Some(integer(name, value, 1..=1000)?),
            _ => return Err(invalid(format!("unknown attribute {name}"))),
        }
    }
    Ok(input)
}

fn queue_param(params: &Params<'_>) -> Result<String, Error> {
    string("QueueName", params.require("QueueName")?).map(str::to_owned)
}

/// Parses one line, without its line terminator.
pub fn parse(line: &[u8]) -> Result<Command, Error> {
    let shape = |reason: &str| Error::new(ErrorCode::InvalidRequest, reason.to_owned());
    let text = std::str::from_utf8(line).map_err(|_| shape("line is not valid UTF-8"))?;
    let value: Value =
        serde_json::from_str(text).map_err(|e| shape(&format!("line is not JSON: {e}")))?;
    let members = value
        .as_object()
        .ok_or_else(|| shape("line is not a JSON object"))?;
    let name = members
        .get("Command")
        .and_then(Value::as_str)
        .ok_or_else(|| shape("Command must be a string"))?;
    let params = Params(members);

    match name {
        "CreateQueue" => {
            let name = params.require("QueueName")?;
            // The attributes are validated after the name, which has its own code.
            let name = QueueName::parse(string("QueueName", name)?)?;
            Ok(Command::CreateQueue {
                name,
                attributes: attributes(params.get("Attributes"))?,
            })
        }
        "SetQueueAttributes" => {
            let queue = params.require("QueueName")?;
            let members = params.require("Attributes")?;
            Ok(Command::SetQueueAttributes {
                queue: string("QueueName", queue)?.to_owned(),
                attributes: attributes(Some(members))?,
            })
        }
        "GetQueueAttributes" => Ok(Command::GetQueueAttributes {
            queue: queue_param(&params)?,
        }),
        "ListQueues" => Ok(Command::ListQueues {
            prefix: optional_string(&params, "Prefix")?.map(str::to_owned),
        }),
        "DeleteQueue" => Ok(Command::DeleteQueue {
            queue: queue_param(&params)?,
        }),
        "PurgeQueue" => Ok(Command::PurgeQueue {
            queue: queue_param(&params)?,
        }),
        "Tick" => {
            let seconds = params.require("Seconds")?;
            Ok(Command::Tick {
                seconds: integer("Seconds", seconds, 0..=1_209_600)?,
            })
        }
        "SendMessage" => {
            let queue = params.require("QueueName")?;
            let body = params.require("Body")?;
            let queue = string("QueueName", queue)?.to_owned();
            let body = string("Body", body)?;
            if !(1..=1024).contains(&body.len()) {
                return Err(invalid("Body must be 1 to 1024 bytes".to_owned()));
            }
            Ok(Command::SendMessage {
                queue,
                body: body.to_owned(),
                delay_seconds: optional_integer(&params, "DelaySeconds", 0..=900)?,
                group: optional_string(&params, "GroupId")?
                    .map(GroupId::parse)
                    .transpose()?,
                deduplication: optional_string(&params, "DeduplicationId")?
                    .map(DedupKey::from_id)
                    .transpose()?,
            })
        }
        "ReceiveMessage" => Ok(Command::ReceiveMessage {
            queue: queue_param(&params)?,
            max_messages: optional_integer(&params, "MaxMessages", 1..=10)?.unwrap_or(1),
            visibility_timeout: optional_integer(&params, "VisibilityTimeout", 0..=43_200)?,
        }),
        "DeleteMessage" => {
            let queue = params.require("QueueName")?;
            let handle = params.require("ReceiptHandle")?;
            Ok(Command::DeleteMessage {
                queue: string("QueueName", queue)?.to_owned(),
                receipt_handle: string("ReceiptHandle", handle)?.to_owned(),
            })
        }
        "ChangeMessageVisibility" => {
            let queue = params.require("QueueName")?;
            let handle = params.require("ReceiptHandle")?;
            let timeout = params.require("VisibilityTimeout")?;
            Ok(Command::ChangeMessageVisibility {
                queue: string("QueueName", queue)?.to_owned(),
                receipt_handle: string("ReceiptHandle", handle)?.to_owned(),
                visibility_timeout: integer("VisibilityTimeout", timeout, 0..=43_200)?,
            })
        }
        other => Err(shape(&format!("unknown command {other:?}"))),
    }
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
            "not json",
            "[]",
            "{}",
            r#"{"Command": 3}"#,
            r#"{"Command": "Nope"}"#,
            r#"{"Command": "Tick", "Seconds": 1} trailing"#,
            r#"{"command": "Tick", "Seconds": 1}"#,
        ];
        for line in cases {
            assert_eq!(code(line), ErrorCode::InvalidRequest, "line {line}");
        }
        assert_eq!(
            parse(b"{\"Command\":\"\xff\"}").unwrap_err().code,
            ErrorCode::InvalidRequest
        );
    }

    #[test]
    fn missing_parameters_are_reported_before_bad_values() {
        let line = r#"{"Command": "SendMessage", "QueueName": 5}"#;
        assert_eq!(code(line), ErrorCode::MissingParameter);
    }

    #[test]
    fn null_counts_as_absent() {
        let line = r#"{"Command": "DeleteQueue", "QueueName": null}"#;
        assert_eq!(code(line), ErrorCode::MissingParameter);
    }

    #[test]
    fn integers_reject_fractions_exponents_and_strings() {
        for seconds in ["1.0", "1e0", "\"1\"", "-1", "1209601", "true"] {
            let line = format!(r#"{{"Command": "Tick", "Seconds": {seconds}}}"#);
            assert_eq!(
                code(&line),
                ErrorCode::InvalidParameterValue,
                "Seconds {seconds}"
            );
        }
        let line = r#"{"Command": "Tick", "Seconds": 1209600}"#;
        assert_eq!(
            parse(line.as_bytes()),
            Ok(Command::Tick { seconds: 1_209_600 })
        );
    }

    #[test]
    fn create_queue_checks_the_name_before_attributes() {
        let line = r#"{"Command": "CreateQueue", "QueueName": "a b", "Attributes": 3}"#;
        assert_eq!(code(line), ErrorCode::QueueNameInvalid);
    }

    #[test]
    fn attributes_must_be_an_object_with_known_valid_members() {
        let cases = [
            r#""x""#,
            r#"{"Bogus": 1}"#,
            r#"{"VisibilityTimeout": 43201}"#,
            r#"{"RetentionSeconds": 59}"#,
            r#"{"Fifo": "true"}"#,
            r#"{"MaxReceiveCount": 0}"#,
            r#"{"DeadLetterQueue": 4}"#,
        ];
        for value in cases {
            let line =
                format!(r#"{{"Command": "CreateQueue", "QueueName": "q", "Attributes": {value}}}"#);
            assert_eq!(
                code(&line),
                ErrorCode::InvalidParameterValue,
                "attributes {value}"
            );
        }
    }

    #[test]
    fn null_attribute_is_absent() {
        let line = r#"{"Command": "CreateQueue", "QueueName": "q", "Attributes": {"Fifo": null}}"#;
        assert_eq!(
            parse(line.as_bytes()),
            Ok(Command::CreateQueue {
                name: QueueName::parse("q").unwrap(),
                attributes: AttributeInput::default(),
            })
        );
    }

    #[test]
    fn send_message_body_is_limited_in_bytes() {
        let ok = format!(
            r#"{{"Command": "SendMessage", "QueueName": "q", "Body": "{}"}}"#,
            "a".repeat(1024)
        );
        assert_eq!(
            parse(ok.as_bytes()),
            Ok(Command::SendMessage {
                queue: "q".to_owned(),
                body: "a".repeat(1024),
                delay_seconds: None,
                group: None,
                deduplication: None,
            })
        );
        // 513 two-byte characters are 1026 bytes.
        let long = format!(
            r#"{{"Command": "SendMessage", "QueueName": "q", "Body": "{}"}}"#,
            "\u{e9}".repeat(513)
        );
        assert_eq!(code(&long), ErrorCode::InvalidParameterValue);
        let empty = r#"{"Command": "SendMessage", "QueueName": "q", "Body": ""}"#;
        assert_eq!(code(empty), ErrorCode::InvalidParameterValue);
    }

    #[test]
    fn receive_message_defaults_to_one_message() {
        let line = r#"{"Command": "ReceiveMessage", "QueueName": "q", "Extra": 1}"#;
        assert_eq!(
            parse(line.as_bytes()),
            Ok(Command::ReceiveMessage {
                queue: "q".to_owned(),
                max_messages: 1,
                visibility_timeout: None,
            })
        );
    }
}
