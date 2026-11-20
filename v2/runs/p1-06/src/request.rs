//! Decoding of one input line into a typed command.
//!
//! Checks run in the order the spec fixes: line shape, then every required
//! parameter, then the value of each present parameter.

use std::fmt::Display;

use serde_json::{Map, Value};

use crate::error::{Error, ErrorCode};
use crate::types::{
    Body, DeduplicationId, Delay, GroupId, MaxMessages, MaxReceiveCount, QueueName, Retention,
    TickSeconds, VisibilityTimeout,
};

/// The queue attributes a request names. Absent and `null` are both `None`.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Attributes {
    pub visibility_timeout: Option<VisibilityTimeout>,
    pub delay: Option<Delay>,
    pub retention: Option<Retention>,
    pub fifo: Option<bool>,
    pub content_based_deduplication: Option<bool>,
    pub dead_letter_queue: Option<String>,
    pub max_receive_count: Option<MaxReceiveCount>,
}

/// The parts of a send that describe the message itself.
#[derive(Debug, PartialEq, Eq)]
pub struct NewMessage {
    pub body: Body,
    pub delay: Option<Delay>,
    pub group: Option<GroupId>,
    pub deduplication_id: Option<DeduplicationId>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    CreateQueue {
        name: QueueName,
        attributes: Attributes,
    },
    SetQueueAttributes {
        queue: String,
        attributes: Attributes,
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
        max_messages: MaxMessages,
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

fn invalid_value(name: &str, reason: impl Display) -> Error {
    Error::new(
        ErrorCode::InvalidParameterValue,
        format!("{name}: {reason}"),
    )
}

fn wrong_type(name: &str, expected: &str) -> Error {
    invalid_value(name, format!("expected {expected}"))
}

fn string<'a>(name: &str, value: &'a Value) -> Result<&'a str, Error> {
    value.as_str().ok_or_else(|| wrong_type(name, "a string"))
}

fn owned_string(name: &str, value: &Value) -> Result<String, Error> {
    string(name, value).map(str::to_owned)
}

fn boolean(name: &str, value: &Value) -> Result<bool, Error> {
    value.as_bool().ok_or_else(|| wrong_type(name, "a boolean"))
}

/// Accepts only numbers written without fraction or exponent.
fn integer(name: &str, value: &Value) -> Result<i64, Error> {
    value.as_i64().ok_or_else(|| {
        if value.is_number() && !value.is_f64() {
            invalid_value(name, "integer out of range")
        } else {
            wrong_type(name, "an integer")
        }
    })
}

fn seconds<const MIN: u32, const MAX: u32>(
    name: &str,
    value: &Value,
) -> Result<crate::types::Seconds<MIN, MAX>, Error> {
    crate::types::Seconds::new(integer(name, value)?).map_err(|e| invalid_value(name, e))
}

fn count<const MIN: u32, const MAX: u32>(
    name: &str,
    value: &Value,
) -> Result<crate::types::Count<MIN, MAX>, Error> {
    crate::types::Count::new(integer(name, value)?).map_err(|e| invalid_value(name, e))
}

fn attributes(name: &str, value: &Value) -> Result<Attributes, Error> {
    let object = value
        .as_object()
        .ok_or_else(|| wrong_type(name, "an object"))?;
    let mut attributes = Attributes::default();
    for (key, value) in object.iter().filter(|(_, value)| !value.is_null()) {
        match key.as_str() {
            "VisibilityTimeout" => attributes.visibility_timeout = Some(seconds(key, value)?),
            "DelaySeconds" => attributes.delay = Some(seconds(key, value)?),
            "RetentionSeconds" => attributes.retention = Some(seconds(key, value)?),
            "Fifo" => attributes.fifo = Some(boolean(key, value)?),
            "ContentBasedDeduplication" => {
                attributes.content_based_deduplication = Some(boolean(key, value)?);
            }
            "DeadLetterQueue" => attributes.dead_letter_queue = Some(owned_string(key, value)?),
            "MaxReceiveCount" => attributes.max_receive_count = Some(count(key, value)?),
            _ => return Err(invalid_value(key, "unknown attribute")),
        }
    }
    Ok(attributes)
}

/// The members of a command object, with `null` read as absent.
struct Params<'a>(&'a Map<String, Value>);

impl<'a> Params<'a> {
    fn value(&self, name: &str) -> Option<&'a Value> {
        self.0.get(name).filter(|value| !value.is_null())
    }

    fn required(&self, name: &str) -> Result<&'a Value, Error> {
        self.value(name)
            .ok_or_else(|| Error::new(ErrorCode::MissingParameter, format!("{name} is required")))
    }

    fn optional<T>(
        &self,
        name: &str,
        decode: impl FnOnce(&str, &'a Value) -> Result<T, Error>,
    ) -> Result<Option<T>, Error> {
        self.value(name)
            .map(|value| decode(name, value))
            .transpose()
    }
}

impl Command {
    pub fn parse(line: &[u8]) -> Result<Self, Error> {
        let invalid_request = |reason: String| Error::new(ErrorCode::InvalidRequest, reason);
        let text = std::str::from_utf8(line)
            .map_err(|e| invalid_request(format!("line is not valid UTF-8: {e}")))?;
        let object: Map<String, Value> = serde_json::from_str(text)
            .map_err(|e| invalid_request(format!("line is not a JSON object: {e}")))?;
        let params = Params(&object);
        let name = params
            .value("Command")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid_request("Command must be a string".to_owned()))?;
        match name {
            "CreateQueue" => {
                let raw_name = params.required("QueueName")?;
                let name = QueueName::parse(string("QueueName", raw_name)?).map_err(|e| {
                    Error::new(ErrorCode::QueueNameInvalid, format!("QueueName: {e}"))
                })?;
                let attributes = params
                    .optional("Attributes", attributes)?
                    .unwrap_or_default();
                Ok(Self::CreateQueue { name, attributes })
            }
            "SetQueueAttributes" => {
                let queue = params.required("QueueName")?;
                let raw_attributes = params.required("Attributes")?;
                Ok(Self::SetQueueAttributes {
                    queue: owned_string("QueueName", queue)?,
                    attributes: attributes("Attributes", raw_attributes)?,
                })
            }
            "GetQueueAttributes" => Ok(Self::GetQueueAttributes {
                queue: owned_string("QueueName", params.required("QueueName")?)?,
            }),
            "ListQueues" => Ok(Self::ListQueues {
                prefix: params.optional("Prefix", owned_string)?,
            }),
            "DeleteQueue" => Ok(Self::DeleteQueue {
                queue: owned_string("QueueName", params.required("QueueName")?)?,
            }),
            "PurgeQueue" => Ok(Self::PurgeQueue {
                queue: owned_string("QueueName", params.required("QueueName")?)?,
            }),
            "Tick" => Ok(Self::Tick {
                seconds: seconds("Seconds", params.required("Seconds")?)?,
            }),
            "SendMessage" => {
                let queue = params.required("QueueName")?;
                let body = params.required("Body")?;
                let queue = owned_string("QueueName", queue)?;
                let body =
                    Body::parse(string("Body", body)?).map_err(|e| invalid_value("Body", e))?;
                let delay = params.optional("DelaySeconds", seconds)?;
                let group = params.optional("GroupId", |name, value| {
                    GroupId::parse(string(name, value)?).map_err(|e| invalid_value(name, e))
                })?;
                let deduplication_id = params.optional("DeduplicationId", |name, value| {
                    DeduplicationId::parse(string(name, value)?).map_err(|e| invalid_value(name, e))
                })?;
                Ok(Self::SendMessage {
                    queue,
                    message: NewMessage {
                        body,
                        delay,
                        group,
                        deduplication_id,
                    },
                })
            }
            "ReceiveMessage" => {
                let queue = owned_string("QueueName", params.required("QueueName")?)?;
                let max_messages = params
                    .optional("MaxMessages", count)?
                    .unwrap_or(MaxMessages::constant(1));
                let visibility_timeout = params.optional("VisibilityTimeout", seconds)?;
                Ok(Self::ReceiveMessage {
                    queue,
                    max_messages,
                    visibility_timeout,
                })
            }
            "DeleteMessage" => {
                let queue = params.required("QueueName")?;
                let handle = params.required("ReceiptHandle")?;
                Ok(Self::DeleteMessage {
                    queue: owned_string("QueueName", queue)?,
                    receipt_handle: owned_string("ReceiptHandle", handle)?,
                })
            }
            "ChangeMessageVisibility" => {
                let queue = params.required("QueueName")?;
                let handle = params.required("ReceiptHandle")?;
                let timeout = params.required("VisibilityTimeout")?;
                Ok(Self::ChangeMessageVisibility {
                    queue: owned_string("QueueName", queue)?,
                    receipt_handle: owned_string("ReceiptHandle", handle)?,
                    visibility_timeout: seconds("VisibilityTimeout", timeout)?,
                })
            }
            other => Err(invalid_request(format!("unknown command {other:?}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(line: &str) -> Result<Command, ErrorCode> {
        Command::parse(line.as_bytes()).map_err(|e| e.code())
    }

    #[test]
    fn invalid_lines_are_invalid_requests() {
        let lines = [
            "",
            "   ",
            "[]",
            "\"Tick\"",
            "{",
            "{} {}",
            "{\"Command\": \"Tick\"} x",
            "{}",
            "{\"Command\": 1}",
            "{\"Command\": null}",
            "{\"Command\": \"tick\"}",
            "{\"Command\": \"Nope\"}",
        ];
        for line in lines {
            assert_eq!(parse(line), Err(ErrorCode::InvalidRequest), "{line:?}");
        }
        assert_eq!(
            Command::parse(b"{\"Command\": \"Tick\", \"x\": \"\xff\"}").map_err(|e| e.code()),
            Err(ErrorCode::InvalidRequest)
        );
    }

    #[test]
    fn required_parameters_are_checked_before_values() {
        let lines = [
            r#"{"Command": "Tick"}"#,
            r#"{"Command": "Tick", "Seconds": null}"#,
            r#"{"Command": "SendMessage", "Body": 5}"#,
            r#"{"Command": "SendMessage", "QueueName": 5}"#,
            r#"{"Command": "SetQueueAttributes", "QueueName": "q"}"#,
            r#"{"Command": "ChangeMessageVisibility", "QueueName": "q", "ReceiptHandle": "r-1"}"#,
            r#"{"Command": "DeleteMessage", "ReceiptHandle": 7}"#,
        ];
        for line in lines {
            assert_eq!(parse(line), Err(ErrorCode::MissingParameter), "{line}");
        }
    }

    #[test]
    fn integers_must_be_written_as_integers() {
        for seconds in [
            "30.0",
            "3e1",
            "\"30\"",
            "true",
            "-1",
            "1209601",
            "18446744073709551616",
        ] {
            let line = format!(r#"{{"Command": "Tick", "Seconds": {seconds}}}"#);
            assert_eq!(
                parse(&line),
                Err(ErrorCode::InvalidParameterValue),
                "{seconds}"
            );
        }
        assert_eq!(
            parse(r#"{"Command": "Tick", "Seconds": 1209600}"#),
            Ok(Command::Tick {
                seconds: TickSeconds::constant(1_209_600)
            })
        );
    }

    #[test]
    fn create_queue_checks_the_name_before_the_attributes() {
        assert_eq!(
            parse(r#"{"Command": "CreateQueue", "QueueName": "a b", "Attributes": 5}"#),
            Err(ErrorCode::QueueNameInvalid)
        );
        assert_eq!(
            parse(r#"{"Command": "CreateQueue", "QueueName": 5, "Attributes": 5}"#),
            Err(ErrorCode::InvalidParameterValue)
        );
        assert_eq!(
            parse(r#"{"Command": "CreateQueue", "QueueName": "q", "Attributes": []}"#),
            Err(ErrorCode::InvalidParameterValue)
        );
    }

    #[test]
    fn attributes_are_decoded_and_null_means_absent() {
        let command = parse(
            r#"{"Command": "CreateQueue", "QueueName": "q.fifo", "Ignored": 1, "Attributes": {
                "VisibilityTimeout": 5, "DelaySeconds": null, "RetentionSeconds": 60,
                "Fifo": true, "ContentBasedDeduplication": false,
                "DeadLetterQueue": "dead.fifo", "MaxReceiveCount": 3, "Unknown": null}}"#,
        );
        assert_eq!(
            command,
            Ok(Command::CreateQueue {
                name: QueueName::parse("q.fifo").unwrap(),
                attributes: Attributes {
                    visibility_timeout: Some(VisibilityTimeout::constant(5)),
                    delay: None,
                    retention: Some(Retention::constant(60)),
                    fifo: Some(true),
                    content_based_deduplication: Some(false),
                    dead_letter_queue: Some("dead.fifo".to_owned()),
                    max_receive_count: Some(MaxReceiveCount::constant(3)),
                }
            })
        );
    }

    #[test]
    fn bad_attributes_are_invalid_parameter_values() {
        let attributes = [
            r#"{"Bogus": 1}"#,
            r#"{"VisibilityTimeout": 43201}"#,
            r#"{"VisibilityTimeout": "5"}"#,
            r#"{"DelaySeconds": 901}"#,
            r#"{"RetentionSeconds": 59}"#,
            r#"{"Fifo": "true"}"#,
            r#"{"ContentBasedDeduplication": 1}"#,
            r#"{"DeadLetterQueue": 1}"#,
            r#"{"MaxReceiveCount": 0}"#,
            r#"{"MaxReceiveCount": 1001}"#,
        ];
        for attributes in attributes {
            let line = format!(
                r#"{{"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {attributes}}}"#
            );
            assert_eq!(
                parse(&line),
                Err(ErrorCode::InvalidParameterValue),
                "{attributes}"
            );
        }
    }

    #[test]
    fn send_message_decodes_every_parameter() {
        assert_eq!(
            parse(
                r#"{"Command": "SendMessage", "QueueName": "q", "Body": "hi",
                    "DelaySeconds": 900, "GroupId": "g", "DeduplicationId": "d"}"#
            ),
            Ok(Command::SendMessage {
                queue: "q".to_owned(),
                message: NewMessage {
                    body: Body::parse("hi").unwrap(),
                    delay: Some(Delay::constant(900)),
                    group: Some(GroupId::parse("g").unwrap()),
                    deduplication_id: Some(DeduplicationId::parse("d").unwrap()),
                }
            })
        );
        let bad = [
            r#""Body": """#,
            r#""Body": 5"#,
            r#""Body": "x", "DelaySeconds": 901"#,
            r#""Body": "x", "GroupId": "a b""#,
            r#""Body": "x", "DeduplicationId": """#,
        ];
        for fields in bad {
            let line = format!(r#"{{"Command": "SendMessage", "QueueName": "q", {fields}}}"#);
            assert_eq!(
                parse(&line),
                Err(ErrorCode::InvalidParameterValue),
                "{fields}"
            );
        }
    }

    #[test]
    fn receive_message_defaults_to_one_message() {
        assert_eq!(
            parse(r#"{"Command": "ReceiveMessage", "QueueName": "q"}"#),
            Ok(Command::ReceiveMessage {
                queue: "q".to_owned(),
                max_messages: MaxMessages::constant(1),
                visibility_timeout: None,
            })
        );
        assert_eq!(
            parse(r#"{"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 11}"#),
            Err(ErrorCode::InvalidParameterValue)
        );
    }
}
