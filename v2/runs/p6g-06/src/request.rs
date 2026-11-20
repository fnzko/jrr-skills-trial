//! Turns one input line into a typed command, applying the error order of
//! section 2: shape, then required parameters, then parameter values.

use std::ops::RangeInclusive;

use serde_json::{Map, Value};

use crate::attributes::AttributePatch;
use crate::error::{Error, ErrorCode};
use crate::ids::{QueueName, is_name_char};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendRequest {
    pub body: String,
    pub delay_seconds: Option<u32>,
    pub group_id: Option<String>,
    pub deduplication_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    CreateQueue {
        name: QueueName,
        attributes: AttributePatch,
    },
    SetQueueAttributes {
        queue: String,
        attributes: AttributePatch,
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
        message: SendRequest,
    },
    ReceiveMessage {
        queue: String,
        max_messages: usize,
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

pub fn parse_line(line: &[u8]) -> Result<Command, Error> {
    let invalid = |why: &str| Error::new(ErrorCode::InvalidRequest, why);
    let text = std::str::from_utf8(line).map_err(|_| invalid("line is not valid UTF-8"))?;
    let members: Map<String, Value> =
        serde_json::from_str(text).map_err(|_| invalid("line is not a JSON object"))?;
    let params = Params(members);
    let name = match params.0.get("Command") {
        Some(Value::String(name)) => name.as_str(),
        _ => return Err(invalid("Command must be a string")),
    };
    match name {
        "CreateQueue" => params.create_queue(),
        "SetQueueAttributes" => params.set_queue_attributes(),
        "GetQueueAttributes" => Ok(Command::GetQueueAttributes {
            queue: params.queue()?,
        }),
        "ListQueues" => Ok(Command::ListQueues {
            prefix: params.optional_string("Prefix")?,
        }),
        "DeleteQueue" => Ok(Command::DeleteQueue {
            queue: params.queue()?,
        }),
        "PurgeQueue" => Ok(Command::PurgeQueue {
            queue: params.queue()?,
        }),
        "Tick" => params.tick(),
        "SendMessage" => params.send_message(),
        "ReceiveMessage" => params.receive_message(),
        "DeleteMessage" => params.delete_message(),
        "ChangeMessageVisibility" => params.change_message_visibility(),
        _ => Err(invalid("unknown command")),
    }
}

struct Params(Map<String, Value>);

impl Params {
    /// A member whose value is `null` counts as absent.
    fn get(&self, name: &str) -> Option<&Value> {
        self.0.get(name).filter(|value| !value.is_null())
    }

    fn require(&self, name: &str) -> Result<&Value, Error> {
        self.get(name).ok_or_else(|| Error::missing(name))
    }

    fn queue(&self) -> Result<String, Error> {
        string("QueueName", self.require("QueueName")?)
    }

    fn optional_string(&self, name: &str) -> Result<Option<String>, Error> {
        self.get(name).map(|value| string(name, value)).transpose()
    }

    fn optional_integer(
        &self,
        name: &str,
        range: RangeInclusive<u32>,
    ) -> Result<Option<u32>, Error> {
        self.get(name)
            .map(|value| integer(name, value, range))
            .transpose()
    }

    fn optional_id(&self, name: &str) -> Result<Option<String>, Error> {
        let id = self.optional_string(name)?;
        match id {
            Some(id) if !(1..=128).contains(&id.len()) || !id.bytes().all(is_name_char) => Err(
                Error::invalid(name, "1 to 128 characters of A-Z a-z 0-9 _ -"),
            ),
            id => Ok(id),
        }
    }

    fn attributes(&self, value: Option<&Value>) -> Result<AttributePatch, Error> {
        let Some(value) = value else {
            return Ok(AttributePatch::default());
        };
        let Value::Object(members) = value else {
            return Err(Error::invalid("Attributes", "an object"));
        };
        let mut patch = AttributePatch::default();
        for (name, value) in members.iter().filter(|(_, value)| !value.is_null()) {
            match name.as_str() {
                "VisibilityTimeout" => {
                    patch.visibility_timeout = Some(integer(name, value, 0..=43_200)?)
                }
                "DelaySeconds" => patch.delay_seconds = Some(integer(name, value, 0..=900)?),
                "RetentionSeconds" => {
                    patch.retention_seconds = Some(integer(name, value, 60..=1_209_600)?)
                }
                "Fifo" => patch.fifo = Some(boolean(name, value)?),
                "ContentBasedDeduplication" => {
                    patch.content_based_deduplication = Some(boolean(name, value)?)
                }
                "DeadLetterQueue" => patch.dead_letter_queue = Some(string(name, value)?),
                "MaxReceiveCount" => {
                    patch.max_receive_count = Some(integer(name, value, 1..=1000)?)
                }
                _ => {
                    return Err(Error::invalid(
                        "Attributes",
                        "made of known attribute names",
                    ));
                }
            }
        }
        Ok(patch)
    }

    fn create_queue(&self) -> Result<Command, Error> {
        let name = self.require("QueueName")?;
        let name = QueueName::parse(&string("QueueName", name)?)?;
        let attributes = self.attributes(self.get("Attributes"))?;
        Ok(Command::CreateQueue { name, attributes })
    }

    fn set_queue_attributes(&self) -> Result<Command, Error> {
        let queue = self.require("QueueName")?;
        let attributes = self.require("Attributes")?;
        Ok(Command::SetQueueAttributes {
            queue: string("QueueName", queue)?,
            attributes: self.attributes(Some(attributes))?,
        })
    }

    fn tick(&self) -> Result<Command, Error> {
        let seconds = integer("Seconds", self.require("Seconds")?, 0..=1_209_600)?;
        Ok(Command::Tick { seconds })
    }

    fn send_message(&self) -> Result<Command, Error> {
        let queue = self.require("QueueName")?;
        let body = self.require("Body")?;
        let queue = string("QueueName", queue)?;
        let body = string("Body", body)?;
        if !(1..=1024).contains(&body.len()) {
            return Err(Error::invalid("Body", "1 to 1024 bytes of UTF-8"));
        }
        let message = SendRequest {
            body,
            delay_seconds: self.optional_integer("DelaySeconds", 0..=900)?,
            group_id: self.optional_id("GroupId")?,
            deduplication_id: self.optional_id("DeduplicationId")?,
        };
        Ok(Command::SendMessage { queue, message })
    }

    fn receive_message(&self) -> Result<Command, Error> {
        let queue = self.queue()?;
        let max_messages = self.optional_integer("MaxMessages", 1..=10)?.unwrap_or(1);
        Ok(Command::ReceiveMessage {
            queue,
            max_messages: max_messages as usize,
            visibility_timeout: self.optional_integer("VisibilityTimeout", 0..=43_200)?,
        })
    }

    fn delete_message(&self) -> Result<Command, Error> {
        let queue = self.require("QueueName")?;
        let handle = self.require("ReceiptHandle")?;
        Ok(Command::DeleteMessage {
            queue: string("QueueName", queue)?,
            receipt_handle: string("ReceiptHandle", handle)?,
        })
    }

    fn change_message_visibility(&self) -> Result<Command, Error> {
        let queue = self.require("QueueName")?;
        let handle = self.require("ReceiptHandle")?;
        let timeout = self.require("VisibilityTimeout")?;
        Ok(Command::ChangeMessageVisibility {
            queue: string("QueueName", queue)?,
            receipt_handle: string("ReceiptHandle", handle)?,
            visibility_timeout: integer("VisibilityTimeout", timeout, 0..=43_200)?,
        })
    }
}

fn string(name: &str, value: &Value) -> Result<String, Error> {
    match value {
        Value::String(text) => Ok(text.clone()),
        _ => Err(Error::invalid(name, "a string")),
    }
}

fn boolean(name: &str, value: &Value) -> Result<bool, Error> {
    value
        .as_bool()
        .ok_or_else(|| Error::invalid(name, "a boolean"))
}

/// Accepts JSON numbers written without fraction or exponent.
fn integer(name: &str, value: &Value, range: RangeInclusive<u32>) -> Result<u32, Error> {
    value
        .as_i64()
        .and_then(|n| u32::try_from(n).ok())
        .filter(|n| range.contains(n))
        .ok_or_else(|| {
            Error::invalid(
                name,
                &format!("an integer from {} to {}", range.start(), range.end()),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code(line: &str) -> ErrorCode {
        parse_line(line.as_bytes()).unwrap_err().code()
    }

    #[test]
    fn request_shape_errors() {
        let cases = [
            "",
            "not json",
            "[]",
            "\"PurgeQueue\"",
            "{}",
            r#"{"Command": 1}"#,
            r#"{"Command": null}"#,
            r#"{"Command": "purgequeue", "QueueName": "q"}"#,
            r#"{"Command": "Nope"}"#,
            r#"{"Command": "Tick", "Seconds": 1} trailing"#,
        ];
        for line in cases {
            assert_eq!(code(line), ErrorCode::InvalidRequest, "{line}");
        }
        assert_eq!(
            parse_line(b"{\"Command\": \"\xff\"}").unwrap_err().code(),
            ErrorCode::InvalidRequest
        );
    }

    #[test]
    fn missing_beats_invalid() {
        let line = r#"{"Command": "SendMessage", "QueueName": 5}"#;
        assert_eq!(code(line), ErrorCode::MissingParameter);
        let line = r#"{"Command": "SendMessage", "QueueName": null, "Body": 5}"#;
        assert_eq!(code(line), ErrorCode::MissingParameter);
    }

    #[test]
    fn integers_must_be_plain_in_range() {
        for seconds in [
            "30.0",
            "3e1",
            "\"30\"",
            "-1",
            "1209601",
            "true",
            "99999999999999999999",
        ] {
            let line = format!(r#"{{"Command": "Tick", "Seconds": {seconds}}}"#);
            assert_eq!(code(&line), ErrorCode::InvalidParameterValue, "{seconds}");
        }
        let parsed = parse_line(br#"{"Command": "Tick", "Seconds": 1209600}"#);
        assert_eq!(parsed, Ok(Command::Tick { seconds: 1_209_600 }));
    }

    #[test]
    fn create_queue_checks_name_before_attributes() {
        let line = r#"{"Command": "CreateQueue", "QueueName": "a b", "Attributes": 3}"#;
        assert_eq!(code(line), ErrorCode::QueueNameInvalid);
        let line = r#"{"Command": "CreateQueue", "QueueName": 7, "Attributes": 3}"#;
        assert_eq!(code(line), ErrorCode::InvalidParameterValue);
        let line = r#"{"Command": "CreateQueue", "QueueName": "q", "Attributes": 3}"#;
        assert_eq!(code(line), ErrorCode::InvalidParameterValue);
    }

    #[test]
    fn attribute_errors() {
        for attributes in [
            r#"{"Unknown": 1}"#,
            r#"{"VisibilityTimeout": "1"}"#,
            r#"{"DelaySeconds": 901}"#,
            r#"{"RetentionSeconds": 59}"#,
            r#"{"Fifo": 1}"#,
            r#"{"DeadLetterQueue": 1}"#,
            r#"{"MaxReceiveCount": 0}"#,
        ] {
            let line = format!(
                r#"{{"Command": "CreateQueue", "QueueName": "q", "Attributes": {attributes}}}"#
            );
            assert_eq!(
                code(&line),
                ErrorCode::InvalidParameterValue,
                "{attributes}"
            );
        }
    }

    #[test]
    fn null_attributes_are_absent() {
        let line = r#"{"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"DelaySeconds": null, "VisibilityTimeout": 5}}"#;
        let Ok(Command::SetQueueAttributes { attributes, .. }) = parse_line(line.as_bytes()) else {
            panic!("expected SetQueueAttributes");
        };
        assert_eq!(
            attributes,
            AttributePatch {
                visibility_timeout: Some(5),
                ..AttributePatch::default()
            }
        );
    }

    #[test]
    fn send_message_parameter_rules() {
        let body = "x".repeat(1025);
        for params in [
            r#""Body": """#.to_string(),
            format!(r#""Body": "{body}""#),
            r#""Body": "ok", "GroupId": """#.to_string(),
            r#""Body": "ok", "GroupId": "a b""#.to_string(),
            r#""Body": "ok", "DeduplicationId": 3"#.to_string(),
            r#""Body": "ok", "DelaySeconds": 901"#.to_string(),
        ] {
            let line = format!(r#"{{"Command": "SendMessage", "QueueName": "q", {params}}}"#);
            assert_eq!(code(&line), ErrorCode::InvalidParameterValue, "{params}");
        }
        let two_byte = "\u{e9}".repeat(512);
        let line =
            format!(r#"{{"Command": "SendMessage", "QueueName": "q", "Body": "{two_byte}"}}"#);
        assert!(parse_line(line.as_bytes()).is_ok());
    }

    #[test]
    fn receive_message_defaults() {
        let parsed = parse_line(br#"{"Command": "ReceiveMessage", "QueueName": "q"}"#);
        assert_eq!(
            parsed,
            Ok(Command::ReceiveMessage {
                queue: "q".into(),
                max_messages: 1,
                visibility_timeout: None
            })
        );
    }
}
