//! Turns one input line into a typed [`Command`], applying the shape, required-parameter and
//! parameter-value checks of the specification in that order.

use std::ops::RangeInclusive;

use serde_json::{Map, Value};

use crate::attributes::AttributePatch;
use crate::error::Error;
use crate::types::{DeduplicationId, GroupId, QueueName, Seconds};

/// A parsed command. Queue names that are looked up stay plain strings: any string is a legal
/// lookup key and simply names no queue if it is not one.
#[derive(Debug, PartialEq)]
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
        seconds: Seconds,
    },
    SendMessage {
        queue: String,
        body: String,
        delay: Option<Seconds>,
        group: Option<GroupId>,
        deduplication: Option<DeduplicationId>,
    },
    ReceiveMessage {
        queue: String,
        max_messages: usize,
        visibility_timeout: Option<Seconds>,
    },
    DeleteMessage {
        queue: String,
        receipt_handle: String,
    },
    ChangeMessageVisibility {
        queue: String,
        receipt_handle: String,
        visibility_timeout: Seconds,
    },
}

const MAX_VISIBILITY_TIMEOUT: u32 = 43_200;
const MAX_DELAY: u32 = 900;
const MAX_BODY_BYTES: usize = 1024;

/// The members of a command object, with `null` members already treated as absent.
struct Params(Map<String, Value>);

impl Params {
    fn take(&mut self, name: &str) -> Option<Value> {
        self.0.remove(name).filter(|value| !value.is_null())
    }

    fn require(&mut self, name: &str) -> Result<Value, Error> {
        self.take(name).ok_or_else(|| Error::missing(name))
    }
}

pub fn parse(line: &[u8]) -> Result<Command, Error> {
    let value: Value = serde_json::from_slice(line)
        .map_err(|error| Error::invalid_request(format!("line is not valid JSON: {error}")))?;
    let Value::Object(members) = value else {
        return Err(Error::invalid_request("line is not a JSON object"));
    };
    let mut params = Params(members);
    let Some(Value::String(name)) = params.take("Command") else {
        return Err(Error::invalid_request("Command must be a string"));
    };
    match name.as_str() {
        "CreateQueue" => {
            let name = params.require("QueueName")?;
            let attributes = params.take("Attributes");
            // The name is checked before every other value so that a bad name gets its own code.
            let name = QueueName::parse(string(name, "QueueName")?)?;
            let attributes = attributes
                .map(attributes_from)
                .transpose()?
                .unwrap_or_default();
            Ok(Command::CreateQueue { name, attributes })
        }
        "SetQueueAttributes" => {
            let queue = params.require("QueueName")?;
            let attributes = params.require("Attributes")?;
            Ok(Command::SetQueueAttributes {
                queue: string(queue, "QueueName")?,
                attributes: attributes_from(attributes)?,
            })
        }
        "GetQueueAttributes" => Ok(Command::GetQueueAttributes {
            queue: queue_name(&mut params)?,
        }),
        "ListQueues" => Ok(Command::ListQueues {
            prefix: params
                .take("Prefix")
                .map(|prefix| string(prefix, "Prefix"))
                .transpose()?,
        }),
        "DeleteQueue" => Ok(Command::DeleteQueue {
            queue: queue_name(&mut params)?,
        }),
        "PurgeQueue" => Ok(Command::PurgeQueue {
            queue: queue_name(&mut params)?,
        }),
        "Tick" => {
            let seconds = params.require("Seconds")?;
            Ok(Command::Tick {
                seconds: seconds_in(&seconds, "Seconds", 1_209_600)?,
            })
        }
        "SendMessage" => parse_send(&mut params),
        "ReceiveMessage" => {
            let queue = params.require("QueueName")?;
            let max_messages = params.take("MaxMessages");
            let visibility_timeout = params.take("VisibilityTimeout");
            Ok(Command::ReceiveMessage {
                queue: string(queue, "QueueName")?,
                max_messages: match max_messages {
                    Some(count) => bounded(&count, "MaxMessages", 1..=10)? as usize,
                    None => 1,
                },
                visibility_timeout: visibility_timeout
                    .map(|v| seconds_in(&v, "VisibilityTimeout", MAX_VISIBILITY_TIMEOUT))
                    .transpose()?,
            })
        }
        "DeleteMessage" => {
            let queue = params.require("QueueName")?;
            let handle = params.require("ReceiptHandle")?;
            Ok(Command::DeleteMessage {
                queue: string(queue, "QueueName")?,
                receipt_handle: string(handle, "ReceiptHandle")?,
            })
        }
        "ChangeMessageVisibility" => {
            let queue = params.require("QueueName")?;
            let handle = params.require("ReceiptHandle")?;
            let timeout = params.require("VisibilityTimeout")?;
            Ok(Command::ChangeMessageVisibility {
                queue: string(queue, "QueueName")?,
                receipt_handle: string(handle, "ReceiptHandle")?,
                visibility_timeout: seconds_in(
                    &timeout,
                    "VisibilityTimeout",
                    MAX_VISIBILITY_TIMEOUT,
                )?,
            })
        }
        other => Err(Error::invalid_request(format!("unknown command {other:?}"))),
    }
}

fn parse_send(params: &mut Params) -> Result<Command, Error> {
    let queue = params.require("QueueName")?;
    let body = params.require("Body")?;
    let delay = params.take("DelaySeconds");
    let group = params.take("GroupId");
    let deduplication = params.take("DeduplicationId");

    let body = string(body, "Body")?;
    if !(1..=MAX_BODY_BYTES).contains(&body.len()) {
        return Err(Error::invalid_value("Body", "1 to 1024 bytes"));
    }
    Ok(Command::SendMessage {
        queue: string(queue, "QueueName")?,
        body,
        delay: delay
            .map(|v| seconds_in(&v, "DelaySeconds", MAX_DELAY))
            .transpose()?,
        group: group
            .map(|v| GroupId::parse(string(v, "GroupId")?))
            .transpose()?,
        deduplication: deduplication
            .map(|v| DeduplicationId::parse(string(v, "DeduplicationId")?))
            .transpose()?,
    })
}

fn queue_name(params: &mut Params) -> Result<String, Error> {
    string(params.require("QueueName")?, "QueueName")
}

fn string(value: Value, name: &str) -> Result<String, Error> {
    match value {
        Value::String(text) => Ok(text),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::Array(_) | Value::Object(_) => {
            Err(Error::invalid_value(name, "a string"))
        }
    }
}

/// An integer in `range`. Numbers written with a fraction or an exponent are not integers.
fn bounded(value: &Value, name: &str, range: RangeInclusive<u32>) -> Result<u32, Error> {
    value
        .as_i64()
        .and_then(|number| u32::try_from(number).ok())
        .filter(|number| range.contains(number))
        .ok_or_else(|| {
            Error::invalid_value(
                name,
                format!("an integer from {} to {}", range.start(), range.end()),
            )
        })
}

fn seconds_in(value: &Value, name: &str, max: u32) -> Result<Seconds, Error> {
    bounded(value, name, 0..=max).map(Seconds)
}

fn boolean(value: &Value, name: &str) -> Result<bool, Error> {
    value
        .as_bool()
        .ok_or_else(|| Error::invalid_value(name, "a boolean"))
}

fn attributes_from(value: Value) -> Result<AttributePatch, Error> {
    let Value::Object(members) = value else {
        return Err(Error::invalid_value("Attributes", "an object"));
    };
    let mut patch = AttributePatch::default();
    for (name, value) in members {
        if value.is_null() {
            continue;
        }
        let name_for_errors = name.as_str();
        match name.as_str() {
            "VisibilityTimeout" => {
                patch.visibility_timeout =
                    Some(seconds_in(&value, name_for_errors, MAX_VISIBILITY_TIMEOUT)?)
            }
            "DelaySeconds" => patch.delay = Some(seconds_in(&value, name_for_errors, MAX_DELAY)?),
            "RetentionSeconds" => {
                patch.retention = Some(Seconds(bounded(&value, name_for_errors, 60..=1_209_600)?))
            }
            "Fifo" => patch.fifo = Some(boolean(&value, name_for_errors)?),
            "ContentBasedDeduplication" => {
                patch.content_based_deduplication = Some(boolean(&value, name_for_errors)?)
            }
            "DeadLetterQueue" => patch.dead_letter_queue = Some(string(value, "DeadLetterQueue")?),
            "MaxReceiveCount" => {
                patch.max_receive_count = Some(bounded(&value, name_for_errors, 1..=1000)?)
            }
            _ => {
                return Err(Error::invalid_value(
                    "Attributes",
                    format!("made of known attributes, not {name:?}"),
                ));
            }
        }
    }
    Ok(patch)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorCode;

    fn error_code(line: &str) -> ErrorCode {
        parse(line.as_bytes()).unwrap_err().code()
    }

    #[test]
    fn malformed_lines_are_invalid_requests() {
        let lines = [
            "",
            "   ",
            "not json",
            "[]",
            "\"PurgeQueue\"",
            "{}",
            r#"{"Command": 5}"#,
            r#"{"Command": null}"#,
            r#"{"Command": "Nope"}"#,
            r#"{"command": "Tick", "Seconds": 1}"#,
            r#"{"Command": "Tick", "Seconds": 1} trailing"#,
        ];
        for line in lines {
            assert_eq!(error_code(line), ErrorCode::InvalidRequest, "{line:?}");
        }
        assert_eq!(
            parse(b"{\"Command\": \"Tick\", \"Seconds\": 1, \"X\": \"\xff\"}")
                .unwrap_err()
                .code(),
            ErrorCode::InvalidRequest
        );
    }

    #[test]
    fn required_parameters_are_checked_before_values() {
        let lines = [
            r#"{"Command": "PurgeQueue"}"#,
            r#"{"Command": "PurgeQueue", "QueueName": null}"#,
            r#"{"Command": "SendMessage", "QueueName": 5}"#,
            r#"{"Command": "DeleteMessage", "QueueName": "q"}"#,
            r#"{"Command": "SetQueueAttributes", "QueueName": 5}"#,
            r#"{"Command": "ChangeMessageVisibility", "QueueName": "q", "ReceiptHandle": "r-1"}"#,
            r#"{"Command": "Tick", "Seconds": null}"#,
        ];
        for line in lines {
            assert_eq!(error_code(line), ErrorCode::MissingParameter, "{line:?}");
        }
    }

    #[test]
    fn wrong_types_and_ranges_are_invalid_values() {
        let lines = [
            r#"{"Command": "PurgeQueue", "QueueName": 5}"#,
            r#"{"Command": "ListQueues", "Prefix": false}"#,
            r#"{"Command": "Tick", "Seconds": 30.0}"#,
            r#"{"Command": "Tick", "Seconds": 3e1}"#,
            r#"{"Command": "Tick", "Seconds": "30"}"#,
            r#"{"Command": "Tick", "Seconds": -1}"#,
            r#"{"Command": "Tick", "Seconds": 1209601}"#,
            r#"{"Command": "Tick", "Seconds": 99999999999999999999}"#,
            r#"{"Command": "SendMessage", "QueueName": "q", "Body": ""}"#,
            r#"{"Command": "SendMessage", "QueueName": "q", "Body": 1}"#,
            r#"{"Command": "SendMessage", "QueueName": "q", "Body": "x", "DelaySeconds": 901}"#,
            r#"{"Command": "SendMessage", "QueueName": "q", "Body": "x", "GroupId": "a b"}"#,
            r#"{"Command": "SendMessage", "QueueName": "q", "Body": "x", "DeduplicationId": ""}"#,
            r#"{"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 0}"#,
            r#"{"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 11}"#,
            r#"{"Command": "ReceiveMessage", "QueueName": "q", "VisibilityTimeout": 43201}"#,
            r#"{"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": []}"#,
            r#"{"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"Unknown": 1}}"#,
            r#"{"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"Fifo": 1}}"#,
            r#"{"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"RetentionSeconds": 59}}"#,
            r#"{"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"MaxReceiveCount": 1001}}"#,
            r#"{"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"DeadLetterQueue": 3}}"#,
        ];
        for line in lines {
            assert_eq!(
                error_code(line),
                ErrorCode::InvalidParameterValue,
                "{line:?}"
            );
        }
    }

    #[test]
    fn body_limit_counts_utf8_bytes() {
        let at_limit = format!(
            r#"{{"Command": "SendMessage", "QueueName": "q", "Body": "{}"}}"#,
            "\u{e9}".repeat(512)
        );
        let over_limit = format!(
            r#"{{"Command": "SendMessage", "QueueName": "q", "Body": "{}"}}"#,
            "\u{e9}".repeat(513)
        );
        assert_eq!(
            parse(at_limit.as_bytes()),
            Ok(Command::SendMessage {
                queue: "q".into(),
                body: "\u{e9}".repeat(512),
                delay: None,
                group: None,
                deduplication: None,
            })
        );
        assert_eq!(error_code(&over_limit), ErrorCode::InvalidParameterValue);
    }

    #[test]
    fn create_queue_checks_the_name_before_anything_else() {
        let bad_name = r#"{"Command": "CreateQueue", "QueueName": "a b", "Attributes": 5}"#;
        assert_eq!(error_code(bad_name), ErrorCode::QueueNameInvalid);
        let bad_type = r#"{"Command": "CreateQueue", "QueueName": 5, "Attributes": 5}"#;
        assert_eq!(error_code(bad_type), ErrorCode::InvalidParameterValue);
        let missing = r#"{"Command": "CreateQueue", "Attributes": 5}"#;
        assert_eq!(error_code(missing), ErrorCode::MissingParameter);
    }

    #[test]
    fn null_members_and_unknown_members_are_ignored() {
        let line = r#"{"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": null,
            "Extra": [1, {"a": null}], "VisibilityTimeout": 7}"#;
        assert_eq!(
            parse(line.as_bytes()),
            Ok(Command::ReceiveMessage {
                queue: "q".into(),
                max_messages: 1,
                visibility_timeout: Some(Seconds(7)),
            })
        );
        let attributes = r#"{"Command": "SetQueueAttributes", "QueueName": "q",
            "Attributes": {"Mystery": null, "DelaySeconds": null, "Fifo": false}}"#;
        assert_eq!(
            parse(attributes.as_bytes()),
            Ok(Command::SetQueueAttributes {
                queue: "q".into(),
                attributes: AttributePatch {
                    fifo: Some(false),
                    ..AttributePatch::default()
                },
            })
        );
    }

    #[test]
    fn send_message_carries_its_optional_parameters() {
        let line = r#"{"Command": "SendMessage", "QueueName": "q.fifo", "Body": "hi",
            "GroupId": "g", "DeduplicationId": "d"}"#;
        assert_eq!(
            parse(line.as_bytes()),
            Ok(Command::SendMessage {
                queue: "q.fifo".into(),
                body: "hi".into(),
                delay: None,
                group: Some(GroupId::parse("g".into()).unwrap()),
                deduplication: Some(DeduplicationId::parse("d".into()).unwrap()),
            })
        );
    }
}
