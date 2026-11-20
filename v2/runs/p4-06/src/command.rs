//! Turns one input line into a typed [`Command`].
//!
//! This is checks 1 to 3 of SPEC section 2: the request shape, the required
//! members, and the value of each present member. Those checks never look at
//! queue state. Checks 4 and 5 (queue lookup and each command's own rules)
//! run later in the broker.
//!
//! Required members are all checked before any value, so a request that is
//! missing one member and has another with the wrong type reports the missing
//! one. The checks inside one command follow the order the spec gives.

use std::ops::RangeInclusive;

use serde_json::{Map, Value};

use crate::attributes::{AttributeUpdates, VISIBILITY_TIMEOUT_RANGE};
use crate::error::{Error, ErrorCode, Result};
use crate::names::{is_name_char, is_valid_queue_name};
use crate::params::{integer, invalid, member, required, string};

pub const BODY_MAX_BYTES: usize = 1024;
pub const TOKEN_MAX_CHARS: usize = 128;
pub const SEND_DELAY_RANGE: RangeInclusive<u64> = 0..=900;
pub const MAX_MESSAGES_RANGE: RangeInclusive<u64> = 1..=10;
pub const TICK_RANGE: RangeInclusive<u64> = 0..=1_209_600;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    CreateQueue {
        name: String,
        attributes: AttributeUpdates,
    },
    SetQueueAttributes {
        name: String,
        attributes: AttributeUpdates,
    },
    GetQueueAttributes {
        name: String,
    },
    ListQueues {
        prefix: Option<String>,
    },
    DeleteQueue {
        name: String,
    },
    PurgeQueue {
        name: String,
    },
    Tick {
        seconds: u64,
    },
    SendMessage(SendMessage),
    ReceiveMessage {
        queue: String,
        max_messages: u64,
        visibility_timeout: Option<u64>,
    },
    DeleteMessage {
        queue: String,
        receipt_handle: String,
    },
    ChangeMessageVisibility {
        queue: String,
        receipt_handle: String,
        visibility_timeout: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendMessage {
    pub queue: String,
    pub body: String,
    pub delay_seconds: Option<u64>,
    pub group_id: Option<String>,
    pub deduplication_id: Option<String>,
}

/// Parses one input line, without its trailing newline.
pub fn parse_line(line: &[u8]) -> Result<Command> {
    let text = std::str::from_utf8(line)
        .map_err(|_| Error::new(ErrorCode::InvalidRequest, "line is not valid UTF-8"))?;
    let value: Value = serde_json::from_str(text).map_err(|e| {
        Error::new(
            ErrorCode::InvalidRequest,
            format!("line is not a JSON value: {e}"),
        )
    })?;
    let object = value
        .as_object()
        .ok_or_else(|| Error::new(ErrorCode::InvalidRequest, "line is not a JSON object"))?;
    let name = member(object, "Command")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::new(ErrorCode::InvalidRequest, "Command must be a string"))?;

    match name {
        "CreateQueue" => create_queue(object),
        "SetQueueAttributes" => set_queue_attributes(object),
        "GetQueueAttributes" => Ok(Command::GetQueueAttributes {
            name: queue_name(object)?,
        }),
        "ListQueues" => list_queues(object),
        "DeleteQueue" => Ok(Command::DeleteQueue {
            name: queue_name(object)?,
        }),
        "PurgeQueue" => Ok(Command::PurgeQueue {
            name: queue_name(object)?,
        }),
        "Tick" => tick(object),
        "SendMessage" => send_message(object).map(Command::SendMessage),
        "ReceiveMessage" => receive_message(object),
        "DeleteMessage" => delete_message(object),
        "ChangeMessageVisibility" => change_message_visibility(object),
        other => Err(Error::new(
            ErrorCode::InvalidRequest,
            format!("unknown command {other}"),
        )),
    }
}

fn create_queue(object: &Map<String, Value>) -> Result<Command> {
    let raw = required(object, "QueueName")?;
    let name = string(raw, "QueueName")?;
    // QueueName is checked before every other parameter (SPEC section 2, check 3).
    if !is_valid_queue_name(name) {
        return Err(Error::new(
            ErrorCode::QueueNameInvalid,
            format!("{name:?} is not a valid queue name"),
        ));
    }
    let attributes = attributes_member(object)?;
    Ok(Command::CreateQueue {
        name: name.to_owned(),
        attributes,
    })
}

fn set_queue_attributes(object: &Map<String, Value>) -> Result<Command> {
    let name_raw = required(object, "QueueName")?;
    let attributes_raw = required(object, "Attributes")?;
    let name = string(name_raw, "QueueName")?.to_owned();
    let attributes = AttributeUpdates::parse(attributes_raw)?;
    Ok(Command::SetQueueAttributes { name, attributes })
}

fn list_queues(object: &Map<String, Value>) -> Result<Command> {
    let prefix = member(object, "Prefix")
        .map(|value| string(value, "Prefix").map(str::to_owned))
        .transpose()?;
    Ok(Command::ListQueues { prefix })
}

fn tick(object: &Map<String, Value>) -> Result<Command> {
    let raw = required(object, "Seconds")?;
    let seconds = integer(raw, "Seconds", TICK_RANGE)?;
    Ok(Command::Tick { seconds })
}

fn send_message(object: &Map<String, Value>) -> Result<SendMessage> {
    let queue_raw = required(object, "QueueName")?;
    let body_raw = required(object, "Body")?;

    let queue = string(queue_raw, "QueueName")?.to_owned();
    let body = string(body_raw, "Body")?;
    if body.is_empty() || body.len() > BODY_MAX_BYTES {
        return Err(invalid(format!(
            "Body must be 1 to {BODY_MAX_BYTES} bytes of UTF-8"
        )));
    }
    let delay_seconds = member(object, "DelaySeconds")
        .map(|value| integer(value, "DelaySeconds", SEND_DELAY_RANGE))
        .transpose()?;
    let group_id = optional_token(object, "GroupId")?;
    let deduplication_id = optional_token(object, "DeduplicationId")?;

    Ok(SendMessage {
        queue,
        body: body.to_owned(),
        delay_seconds,
        group_id,
        deduplication_id,
    })
}

fn receive_message(object: &Map<String, Value>) -> Result<Command> {
    let queue = queue_name(object)?;
    let max_messages = member(object, "MaxMessages")
        .map(|value| integer(value, "MaxMessages", MAX_MESSAGES_RANGE))
        .transpose()?
        .unwrap_or(1);
    let visibility_timeout = member(object, "VisibilityTimeout")
        .map(|value| integer(value, "VisibilityTimeout", VISIBILITY_TIMEOUT_RANGE))
        .transpose()?;
    Ok(Command::ReceiveMessage {
        queue,
        max_messages,
        visibility_timeout,
    })
}

fn delete_message(object: &Map<String, Value>) -> Result<Command> {
    let queue_raw = required(object, "QueueName")?;
    let handle_raw = required(object, "ReceiptHandle")?;
    Ok(Command::DeleteMessage {
        queue: string(queue_raw, "QueueName")?.to_owned(),
        receipt_handle: string(handle_raw, "ReceiptHandle")?.to_owned(),
    })
}

fn change_message_visibility(object: &Map<String, Value>) -> Result<Command> {
    let queue_raw = required(object, "QueueName")?;
    let handle_raw = required(object, "ReceiptHandle")?;
    let timeout_raw = required(object, "VisibilityTimeout")?;
    Ok(Command::ChangeMessageVisibility {
        queue: string(queue_raw, "QueueName")?.to_owned(),
        receipt_handle: string(handle_raw, "ReceiptHandle")?.to_owned(),
        visibility_timeout: integer(timeout_raw, "VisibilityTimeout", VISIBILITY_TIMEOUT_RANGE)?,
    })
}

fn queue_name(object: &Map<String, Value>) -> Result<String> {
    let raw = required(object, "QueueName")?;
    Ok(string(raw, "QueueName")?.to_owned())
}

fn attributes_member(object: &Map<String, Value>) -> Result<AttributeUpdates> {
    match member(object, "Attributes") {
        Some(value) => AttributeUpdates::parse(value),
        None => Ok(AttributeUpdates::default()),
    }
}

/// A `GroupId` or `DeduplicationId`: 1 to 128 characters from the name alphabet.
fn optional_token(object: &Map<String, Value>, name: &str) -> Result<Option<String>> {
    let Some(value) = member(object, name) else {
        return Ok(None);
    };
    let token = string(value, name)?;
    let well_formed =
        (1..=TOKEN_MAX_CHARS).contains(&token.len()) && token.chars().all(is_name_char);
    if !well_formed {
        return Err(invalid(format!(
            "{name} must be 1 to {TOKEN_MAX_CHARS} characters from A-Z a-z 0-9 _ -"
        )));
    }
    Ok(Some(token.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attributes::AttributeUpdates;

    fn parse(text: &str) -> Result<Command> {
        parse_line(text.as_bytes())
    }

    fn code(text: &str) -> ErrorCode {
        parse(text).unwrap_err().code
    }

    #[test]
    fn shape_errors_are_invalid_requests() {
        assert_eq!(
            parse_line(&[0xff, 0xfe]).unwrap_err().code,
            ErrorCode::InvalidRequest
        );
        assert_eq!(code(""), ErrorCode::InvalidRequest);
        assert_eq!(code("   "), ErrorCode::InvalidRequest);
        assert_eq!(code("[]"), ErrorCode::InvalidRequest);
        assert_eq!(code("{}"), ErrorCode::InvalidRequest);
        assert_eq!(
            code(r#"{"command": "Tick", "Seconds": 1}"#),
            ErrorCode::InvalidRequest
        );
        assert_eq!(code(r#"{"Command": 5}"#), ErrorCode::InvalidRequest);
        assert_eq!(code(r#"{"Command": null}"#), ErrorCode::InvalidRequest);
        assert_eq!(code(r#"{"Command": "Nope"}"#), ErrorCode::InvalidRequest);
        assert_eq!(
            code(r#"{"Command": "Tick"} trailing"#),
            ErrorCode::InvalidRequest
        );
    }

    #[test]
    fn missing_required_beats_wrong_type() {
        // QueueName is absent and Body has the wrong type: check 2 decides.
        assert_eq!(
            code(r#"{"Command": "SendMessage", "Body": 5}"#),
            ErrorCode::MissingParameter
        );
        assert_eq!(
            code(
                r#"{"Command": "ChangeMessageVisibility", "QueueName": 1, "ReceiptHandle": "r-1"}"#
            ),
            ErrorCode::MissingParameter,
            "VisibilityTimeout is required; QueueName's wrong type is check 3"
        );
    }

    #[test]
    fn null_required_member_is_missing() {
        assert_eq!(
            code(r#"{"Command": "Tick", "Seconds": null}"#),
            ErrorCode::MissingParameter
        );
    }

    #[test]
    fn create_queue_checks_its_name_before_other_parameters() {
        assert_eq!(
            code(r#"{"Command": "CreateQueue", "QueueName": "a b", "Attributes": 5}"#),
            ErrorCode::QueueNameInvalid
        );
        assert_eq!(
            code(r#"{"Command": "CreateQueue", "QueueName": 5}"#),
            ErrorCode::InvalidParameterValue
        );
        assert_eq!(
            code(r#"{"Command": "CreateQueue", "QueueName": "ok", "Attributes": 5}"#),
            ErrorCode::InvalidParameterValue
        );
    }

    #[test]
    fn other_commands_accept_any_string_as_a_queue_name() {
        // Checks 4 decide these, so the syntax is not checked here.
        let command = parse(r#"{"Command": "GetQueueAttributes", "QueueName": "a b.c"}"#).unwrap();
        assert_eq!(
            command,
            Command::GetQueueAttributes {
                name: "a b.c".into()
            }
        );
    }

    #[test]
    fn send_message_checks_body_and_tokens() {
        let long_body = "x".repeat(1025);
        let text = format!(r#"{{"Command":"SendMessage","QueueName":"q","Body":"{long_body}"}}"#);
        assert_eq!(code(&text), ErrorCode::InvalidParameterValue);
        assert_eq!(
            code(r#"{"Command":"SendMessage","QueueName":"q","Body":""}"#),
            ErrorCode::InvalidParameterValue
        );
        let max_body = "é".repeat(512); // 1024 bytes
        let text = format!(r#"{{"Command":"SendMessage","QueueName":"q","Body":"{max_body}"}}"#);
        assert!(parse(&text).is_ok());
        assert_eq!(
            code(r#"{"Command":"SendMessage","QueueName":"q","Body":"b","GroupId":"a.b"}"#),
            ErrorCode::InvalidParameterValue
        );
        assert_eq!(
            code(r#"{"Command":"SendMessage","QueueName":"q","Body":"b","DelaySeconds":30.0}"#),
            ErrorCode::InvalidParameterValue
        );
    }

    #[test]
    fn receive_defaults_and_bounds() {
        assert_eq!(
            parse(r#"{"Command": "ReceiveMessage", "QueueName": "q"}"#).unwrap(),
            Command::ReceiveMessage {
                queue: "q".into(),
                max_messages: 1,
                visibility_timeout: None
            }
        );
        assert_eq!(
            code(r#"{"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 11}"#),
            ErrorCode::InvalidParameterValue
        );
    }

    #[test]
    fn list_queues_treats_null_prefix_as_absent() {
        assert_eq!(
            parse(r#"{"Command": "ListQueues", "Prefix": null}"#).unwrap(),
            Command::ListQueues { prefix: None }
        );
    }

    #[test]
    fn create_queue_reads_attributes() {
        let command = parse(
            r#"{"Command": "CreateQueue", "QueueName": "q", "Attributes": {"DelaySeconds": 5}}"#,
        )
        .unwrap();
        let expected = AttributeUpdates {
            delay_seconds: Some(5),
            ..AttributeUpdates::default()
        };
        assert_eq!(
            command,
            Command::CreateQueue {
                name: "q".into(),
                attributes: expected
            }
        );
    }

    #[test]
    fn unknown_members_are_ignored() {
        assert_eq!(
            parse(r#"{"Command": "Tick", "Seconds": 3, "Extra": [1]}"#).unwrap(),
            Command::Tick { seconds: 3 }
        );
    }
}
