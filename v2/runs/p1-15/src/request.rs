//! Turns one input line into a [`Command`], applying the shape, required-parameter and
//! parameter-value checks in the order the spec fixes.

use std::ops::RangeInclusive;

use serde_json::{Map, Value};

use crate::attributes::AttributePatch;
use crate::error::{Error, ErrorCode, Result};

/// A message as the client sends it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewMessage {
    pub body: String,
    pub delay_seconds: Option<u32>,
    pub group_id: Option<String>,
    pub deduplication_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    CreateQueue {
        name: String,
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
        message: NewMessage,
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

pub fn parse(line: &[u8]) -> Result<Command> {
    let text = std::str::from_utf8(line)
        .map_err(|e| invalid_request(format!("line is not valid UTF-8: {e}")))?;
    let value: Value = serde_json::from_str(text)
        .map_err(|e| invalid_request(format!("line is not valid JSON: {e}")))?;
    let Value::Object(members) = value else {
        return Err(invalid_request("line is not a JSON object"));
    };
    let mut params = Params(members);
    let Some(Value::String(name)) = params.0.remove("Command") else {
        return Err(invalid_request("Command is missing or not a string"));
    };
    match name.as_str() {
        "CreateQueue" => create_queue(&mut params),
        "SetQueueAttributes" => set_queue_attributes(&mut params),
        "GetQueueAttributes" => Ok(Command::GetQueueAttributes {
            queue: params.queue()?,
        }),
        "ListQueues" => Ok(Command::ListQueues {
            prefix: params.optional("Prefix", string)?,
        }),
        "DeleteQueue" => Ok(Command::DeleteQueue {
            queue: params.queue()?,
        }),
        "PurgeQueue" => Ok(Command::PurgeQueue {
            queue: params.queue()?,
        }),
        "Tick" => {
            let seconds = params.require("Seconds")?;
            Ok(Command::Tick {
                seconds: int("Seconds", seconds, 0..=1_209_600)?,
            })
        }
        "SendMessage" => send_message(&mut params),
        "ReceiveMessage" => receive_message(&mut params),
        "DeleteMessage" => {
            let (queue, receipt_handle) = params.queue_and_handle()?;
            Ok(Command::DeleteMessage {
                queue,
                receipt_handle,
            })
        }
        "ChangeMessageVisibility" => {
            let queue = params.require("QueueName")?;
            let handle = params.require("ReceiptHandle")?;
            let timeout = params.require("VisibilityTimeout")?;
            Ok(Command::ChangeMessageVisibility {
                queue: string("QueueName", queue)?,
                receipt_handle: string("ReceiptHandle", handle)?,
                visibility_timeout: int("VisibilityTimeout", timeout, 0..=43_200)?,
            })
        }
        other => Err(invalid_request(format!("unknown command {other:?}"))),
    }
}

fn create_queue(params: &mut Params) -> Result<Command> {
    let name = params.require("QueueName")?;
    let attributes = params.take("Attributes");
    // Unlike every other parameter, the name is judged first and has its own error code.
    let name = string("QueueName", name)?;
    if !is_valid_queue_name(&name) {
        return Err(Error::new(
            ErrorCode::QueueNameInvalid,
            format!("{name:?} is not a valid queue name"),
        ));
    }
    let attributes = attributes
        .map(attributes_patch)
        .transpose()?
        .unwrap_or_default();
    Ok(Command::CreateQueue { name, attributes })
}

fn set_queue_attributes(params: &mut Params) -> Result<Command> {
    let queue = params.require("QueueName")?;
    let attributes = params.require("Attributes")?;
    Ok(Command::SetQueueAttributes {
        queue: string("QueueName", queue)?,
        attributes: attributes_patch(attributes)?,
    })
}

fn send_message(params: &mut Params) -> Result<Command> {
    let queue = params.require("QueueName")?;
    let body = params.require("Body")?;
    let queue = string("QueueName", queue)?;
    let body = string("Body", body)?;
    if !(1..=1024).contains(&body.len()) {
        return Err(invalid_value("Body must be 1 to 1024 bytes"));
    }
    let delay_seconds = params.optional("DelaySeconds", |n, v| int(n, v, 0..=900))?;
    let group_id = params.optional("GroupId", identifier)?;
    let deduplication_id = params.optional("DeduplicationId", identifier)?;
    Ok(Command::SendMessage {
        queue,
        message: NewMessage {
            body,
            delay_seconds,
            group_id,
            deduplication_id,
        },
    })
}

fn receive_message(params: &mut Params) -> Result<Command> {
    let queue = params.queue()?;
    let max_messages = params
        .optional("MaxMessages", |n, v| int(n, v, 1..=10))?
        .unwrap_or(1);
    let visibility_timeout = params.optional("VisibilityTimeout", |n, v| int(n, v, 0..=43_200))?;
    Ok(Command::ReceiveMessage {
        queue,
        max_messages,
        visibility_timeout,
    })
}

fn attributes_patch(value: Value) -> Result<AttributePatch> {
    let Value::Object(members) = value else {
        return Err(invalid_value("Attributes must be an object"));
    };
    let mut patch = AttributePatch::default();
    for (name, value) in members {
        if value.is_null() {
            continue;
        }
        match name.as_str() {
            "VisibilityTimeout" => patch.visibility_timeout = Some(int(&name, value, 0..=43_200)?),
            "DelaySeconds" => patch.delay_seconds = Some(int(&name, value, 0..=900)?),
            "RetentionSeconds" => {
                patch.retention_seconds = Some(int(&name, value, 60..=1_209_600)?);
            }
            "Fifo" => patch.fifo = Some(boolean(&name, value)?),
            "ContentBasedDeduplication" => {
                patch.content_based_deduplication = Some(boolean(&name, value)?);
            }
            "DeadLetterQueue" => patch.dead_letter_queue = Some(string(&name, value)?),
            "MaxReceiveCount" => patch.max_receive_count = Some(int(&name, value, 1..=1000)?),
            _ => return Err(invalid_value(format!("unknown attribute {name:?}"))),
        }
    }
    Ok(patch)
}

/// The members of a command object, with `null` members counted as absent.
struct Params(Map<String, Value>);

impl Params {
    fn take(&mut self, name: &str) -> Option<Value> {
        self.0.remove(name).filter(|v| !v.is_null())
    }

    fn require(&mut self, name: &str) -> Result<Value> {
        self.take(name)
            .ok_or_else(|| Error::new(ErrorCode::MissingParameter, format!("{name} is required")))
    }

    fn optional<T>(
        &mut self,
        name: &str,
        convert: impl FnOnce(&str, Value) -> Result<T>,
    ) -> Result<Option<T>> {
        self.take(name).map(|v| convert(name, v)).transpose()
    }

    fn queue(&mut self) -> Result<String> {
        let queue = self.require("QueueName")?;
        string("QueueName", queue)
    }

    fn queue_and_handle(&mut self) -> Result<(String, String)> {
        // Both are required before either is type-checked.
        let queue = self.require("QueueName")?;
        let handle = self.require("ReceiptHandle")?;
        Ok((
            string("QueueName", queue)?,
            string("ReceiptHandle", handle)?,
        ))
    }
}

fn string(name: &str, value: Value) -> Result<String> {
    match value {
        Value::String(s) => Ok(s),
        _ => Err(invalid_value(format!("{name} must be a string"))),
    }
}

fn boolean(name: &str, value: Value) -> Result<bool> {
    value
        .as_bool()
        .ok_or_else(|| invalid_value(format!("{name} must be a boolean")))
}

/// An integer written without fraction or exponent, inside `range`.
fn int(name: &str, value: Value, range: RangeInclusive<u32>) -> Result<u32> {
    value
        .as_u64()
        .and_then(|n| u32::try_from(n).ok())
        .filter(|n| range.contains(n))
        .ok_or_else(|| {
            invalid_value(format!(
                "{name} must be an integer from {} to {}",
                range.start(),
                range.end()
            ))
        })
}

/// A group or deduplication id: 1 to 128 characters of the queue-name alphabet.
fn identifier(name: &str, value: Value) -> Result<String> {
    let s = string(name, value)?;
    if (1..=128).contains(&s.len()) && s.bytes().all(is_name_byte) {
        Ok(s)
    } else {
        Err(invalid_value(format!(
            "{name} must be 1 to 128 characters from A-Z a-z 0-9 _ -"
        )))
    }
}

fn is_name_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-'
}

pub fn is_valid_queue_name(name: &str) -> bool {
    let stem = name.strip_suffix(".fifo").unwrap_or(name);
    let stem_limit = if stem.len() == name.len() { 80 } else { 75 };
    (1..=stem_limit).contains(&stem.len()) && stem.bytes().all(is_name_byte)
}

fn invalid_request(message: impl Into<String>) -> Error {
    Error::new(ErrorCode::InvalidRequest, message)
}

fn invalid_value(message: impl Into<String>) -> Error {
    Error::new(ErrorCode::InvalidParameterValue, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code(line: &str) -> ErrorCode {
        parse(line.as_bytes()).unwrap_err().code
    }

    #[test]
    fn shape_errors_are_invalid_request() {
        for line in [
            "",
            "[]",
            "{",
            r#"{"Command": 5}"#,
            r#"{"Command": "Nope"}"#,
            r#"{} x"#,
            r#"{"command":"Tick","Seconds":1}"#,
        ] {
            assert_eq!(code(line), ErrorCode::InvalidRequest, "line {line:?}");
        }
        assert_eq!(
            parse(b"{\"Command\":\"Tick\",\"Seconds\":1}\xff")
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }

    #[test]
    fn missing_beats_invalid_value() {
        assert_eq!(
            code(r#"{"Command":"SendMessage","Body":5}"#),
            ErrorCode::MissingParameter
        );
        assert_eq!(
            code(r#"{"Command":"SendMessage","QueueName":null,"Body":"x"}"#),
            ErrorCode::MissingParameter
        );
    }

    #[test]
    fn integers_must_be_plain_and_in_range() {
        for seconds in ["30.0", "3e1", r#""30""#, "-1", "1209601", "true"] {
            let line = format!(r#"{{"Command":"Tick","Seconds":{seconds}}}"#);
            assert_eq!(
                code(&line),
                ErrorCode::InvalidParameterValue,
                "Seconds {seconds}"
            );
        }
        assert_eq!(
            parse(br#"{"Command":"Tick","Seconds":1209600}"#),
            Ok(Command::Tick { seconds: 1_209_600 })
        );
    }

    #[test]
    fn create_queue_judges_the_name_before_attributes() {
        assert_eq!(
            code(r#"{"Command":"CreateQueue","QueueName":"a b","Attributes":5}"#),
            ErrorCode::QueueNameInvalid
        );
        assert_eq!(
            code(r#"{"Command":"CreateQueue","QueueName":"ab","Attributes":5}"#),
            ErrorCode::InvalidParameterValue
        );
        assert_eq!(
            code(r#"{"Command":"CreateQueue","QueueName":"ab","Attributes":{"Bogus":1}}"#),
            ErrorCode::InvalidParameterValue
        );
    }

    #[test]
    fn null_attributes_are_absent() {
        assert_eq!(
            parse(
                br#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"DelaySeconds":null}}"#
            ),
            Ok(Command::CreateQueue {
                name: "q".into(),
                attributes: AttributePatch::default()
            })
        );
    }

    #[test]
    fn queue_name_syntax() {
        let long = "a".repeat(80);
        let too_long = "a".repeat(81);
        let fifo_max = format!("{}.fifo", "a".repeat(75));
        let fifo_too_long = format!("{}.fifo", "a".repeat(76));
        for (name, valid) in [
            ("a", true),
            ("A_z-9", true),
            (long.as_str(), true),
            (too_long.as_str(), false),
            ("q.fifo", true),
            (fifo_max.as_str(), true),
            (fifo_too_long.as_str(), false),
            ("", false),
            (".fifo", false),
            ("a.b", false),
            ("a.fifo.fifo", false),
            ("é", false),
        ] {
            assert_eq!(is_valid_queue_name(name), valid, "name {name:?}");
        }
    }

    #[test]
    fn send_message_checks_body_and_ids() {
        let body_ok = format!(
            r#"{{"Command":"SendMessage","QueueName":"q","Body":"{}"}}"#,
            "é".repeat(512)
        );
        assert!(parse(body_ok.as_bytes()).is_ok());
        let body_big = format!(
            r#"{{"Command":"SendMessage","QueueName":"q","Body":"{}"}}"#,
            "é".repeat(513)
        );
        assert_eq!(code(&body_big), ErrorCode::InvalidParameterValue);
        assert_eq!(
            code(r#"{"Command":"SendMessage","QueueName":"q","Body":""}"#),
            ErrorCode::InvalidParameterValue
        );
        assert_eq!(
            code(r#"{"Command":"SendMessage","QueueName":"q","Body":"x","GroupId":"a b"}"#),
            ErrorCode::InvalidParameterValue
        );
    }
}
