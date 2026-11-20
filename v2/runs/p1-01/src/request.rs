//! Turns one input line into a typed [`Command`].
//!
//! This covers steps 1 to 3 of the error order in the spec: request shape,
//! required parameters, then parameter values. Everything that needs queue
//! state happens later, in the broker.

use std::ops::RangeInclusive;

use serde_json::{Map, Value};

use crate::error::{Error, ErrorCode};

const VISIBILITY_TIMEOUT: RangeInclusive<u64> = 0..=43_200;
const DELAY_SECONDS: RangeInclusive<u64> = 0..=900;
const RETENTION_SECONDS: RangeInclusive<u64> = 60..=1_209_600;
const MAX_RECEIVE_COUNT: RangeInclusive<u64> = 1..=1000;
const MAX_MESSAGES: RangeInclusive<u64> = 1..=10;
const TICK_SECONDS: RangeInclusive<u64> = 0..=1_209_600;
const BODY_BYTES: RangeInclusive<usize> = 1..=1024;
const IDENTIFIER_LEN: RangeInclusive<usize> = 1..=128;

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    CreateQueue {
        name: String,
        attributes: AttributeUpdate,
    },
    SetQueueAttributes {
        queue: String,
        attributes: AttributeUpdate,
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

#[derive(Debug, PartialEq, Eq)]
pub struct SendMessage {
    pub queue: String,
    pub body: String,
    pub delay_seconds: Option<u64>,
    pub group_id: Option<String>,
    pub deduplication_id: Option<String>,
}

/// The queue attributes given in a request, each one optional.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct AttributeUpdate {
    pub visibility_timeout: Option<u64>,
    pub delay_seconds: Option<u64>,
    pub retention_seconds: Option<u64>,
    pub fifo: Option<bool>,
    pub content_based_deduplication: Option<bool>,
    pub dead_letter_queue: Option<String>,
    pub max_receive_count: Option<u64>,
}

impl AttributeUpdate {
    fn parse(members: &Map<String, Value>) -> Result<Self, Error> {
        let mut update = Self::default();
        for (name, value) in members.iter().filter(|(_, value)| !value.is_null()) {
            match name.as_str() {
                "VisibilityTimeout" => {
                    update.visibility_timeout = Some(integer(name, value, VISIBILITY_TIMEOUT)?);
                }
                "DelaySeconds" => update.delay_seconds = Some(integer(name, value, DELAY_SECONDS)?),
                "RetentionSeconds" => {
                    update.retention_seconds = Some(integer(name, value, RETENTION_SECONDS)?);
                }
                "Fifo" => update.fifo = Some(boolean(name, value)?),
                "ContentBasedDeduplication" => {
                    update.content_based_deduplication = Some(boolean(name, value)?);
                }
                "DeadLetterQueue" => {
                    update.dead_letter_queue = Some(string(name, value)?.to_owned())
                }
                "MaxReceiveCount" => {
                    update.max_receive_count = Some(integer(name, value, MAX_RECEIVE_COUNT)?);
                }
                _ => return Err(Error::invalid_value(format!("unknown attribute {name:?}"))),
            }
        }
        Ok(update)
    }

    /// The dead-letter configuration, which is either given whole or not at all.
    pub fn dead_letter(&self) -> Result<Option<DeadLetter>, Error> {
        match (&self.dead_letter_queue, self.max_receive_count) {
            (Some(queue), Some(max_receive_count)) => Ok(Some(DeadLetter {
                queue: queue.clone(),
                max_receive_count,
            })),
            (None, None) => Ok(None),
            _ => Err(Error::invalid_value(
                "DeadLetterQueue and MaxReceiveCount must be given together",
            )),
        }
    }
}

/// Where messages go after too many deliveries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeadLetter {
    pub queue: String,
    pub max_receive_count: u64,
}

pub fn parse(line: &[u8]) -> Result<Command, Error> {
    let value: Value = serde_json::from_slice(line)
        .map_err(|e| invalid_request(format!("line is not valid JSON: {e}")))?;
    let Value::Object(members) = &value else {
        return Err(invalid_request("line is not a JSON object"));
    };
    let Some(Value::String(name)) = members.get("Command") else {
        return Err(invalid_request("Command must be a string"));
    };
    let params = Params(members);
    match name.as_str() {
        "CreateQueue" => create_queue(&params),
        "SetQueueAttributes" => set_queue_attributes(&params),
        "GetQueueAttributes" => Ok(Command::GetQueueAttributes {
            queue: queue_name(&params)?,
        }),
        "ListQueues" => Ok(Command::ListQueues {
            prefix: params.optional("Prefix", string)?.map(str::to_owned),
        }),
        "DeleteQueue" => Ok(Command::DeleteQueue {
            queue: queue_name(&params)?,
        }),
        "PurgeQueue" => Ok(Command::PurgeQueue {
            queue: queue_name(&params)?,
        }),
        "Tick" => {
            params.require_all(&["Seconds"])?;
            let seconds = params.required("Seconds", |n, v| integer(n, v, TICK_SECONDS))?;
            Ok(Command::Tick { seconds })
        }
        "SendMessage" => send_message(&params),
        "ReceiveMessage" => receive_message(&params),
        "DeleteMessage" => {
            params.require_all(&["QueueName", "ReceiptHandle"])?;
            Ok(Command::DeleteMessage {
                queue: queue_name(&params)?,
                receipt_handle: params.required("ReceiptHandle", string)?.to_owned(),
            })
        }
        "ChangeMessageVisibility" => {
            params.require_all(&["QueueName", "ReceiptHandle", "VisibilityTimeout"])?;
            Ok(Command::ChangeMessageVisibility {
                queue: queue_name(&params)?,
                receipt_handle: params.required("ReceiptHandle", string)?.to_owned(),
                visibility_timeout: params.required("VisibilityTimeout", |n, v| {
                    integer(n, v, VISIBILITY_TIMEOUT)
                })?,
            })
        }
        other => Err(invalid_request(format!("unknown command {other:?}"))),
    }
}

fn create_queue(params: &Params) -> Result<Command, Error> {
    let name = params.required("QueueName", string)?;
    if !is_valid_queue_name(name) {
        return Err(Error::new(
            ErrorCode::QueueNameInvalid,
            format!("{name:?} is not a valid queue name"),
        ));
    }
    Ok(Command::CreateQueue {
        name: name.to_owned(),
        attributes: attributes(params)?.unwrap_or_default(),
    })
}

fn set_queue_attributes(params: &Params) -> Result<Command, Error> {
    params.require_all(&["QueueName", "Attributes"])?;
    Ok(Command::SetQueueAttributes {
        queue: queue_name(params)?,
        attributes: attributes(params)?.unwrap_or_default(),
    })
}

fn send_message(params: &Params) -> Result<Command, Error> {
    params.require_all(&["QueueName", "Body"])?;
    let queue = queue_name(params)?;
    let body = params.required("Body", |n, v| {
        let body = string(n, v)?;
        if BODY_BYTES.contains(&body.len()) {
            Ok(body)
        } else {
            Err(Error::invalid_value("Body must be 1 to 1024 bytes"))
        }
    })?;
    Ok(Command::SendMessage(SendMessage {
        queue,
        body: body.to_owned(),
        delay_seconds: params.optional("DelaySeconds", |n, v| integer(n, v, DELAY_SECONDS))?,
        group_id: params.optional("GroupId", identifier)?.map(str::to_owned),
        deduplication_id: params
            .optional("DeduplicationId", identifier)?
            .map(str::to_owned),
    }))
}

fn receive_message(params: &Params) -> Result<Command, Error> {
    Ok(Command::ReceiveMessage {
        queue: queue_name(params)?,
        max_messages: params
            .optional("MaxMessages", |n, v| integer(n, v, MAX_MESSAGES))?
            .unwrap_or(1),
        visibility_timeout: params.optional("VisibilityTimeout", |n, v| {
            integer(n, v, VISIBILITY_TIMEOUT)
        })?,
    })
}

/// `QueueName`, for commands where it is the only required parameter or the others were
/// already checked with `require_all`.
fn queue_name(params: &Params) -> Result<String, Error> {
    Ok(params.required("QueueName", string)?.to_owned())
}

fn attributes(params: &Params) -> Result<Option<AttributeUpdate>, Error> {
    params.optional("Attributes", |name, value| match value {
        Value::Object(members) => AttributeUpdate::parse(members),
        _ => Err(wrong_type(name, "an object")),
    })
}

/// A queue name is 1 to 80 characters of `A-Z a-z 0-9 _ -`, or 1 to 75 of them plus `.fifo`.
fn is_valid_queue_name(name: &str) -> bool {
    let stem = name.strip_suffix(".fifo").unwrap_or(name);
    let max_stem = if stem.len() == name.len() { 80 } else { 75 };
    (1..=max_stem).contains(&stem.len()) && stem.bytes().all(is_name_byte)
}

fn is_name_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-'
}

struct Params<'a>(&'a Map<String, Value>);

impl<'a> Params<'a> {
    /// A member counts as absent when it is `null`.
    fn get(&self, name: &str) -> Option<&'a Value> {
        self.0.get(name).filter(|value| !value.is_null())
    }

    fn require_all(&self, names: &[&str]) -> Result<(), Error> {
        match names.iter().find(|name| self.get(name).is_none()) {
            Some(name) => Err(Error::missing(name)),
            None => Ok(()),
        }
    }

    fn required<T>(
        &self,
        name: &str,
        convert: impl FnOnce(&str, &'a Value) -> Result<T, Error>,
    ) -> Result<T, Error> {
        convert(name, self.get(name).ok_or_else(|| Error::missing(name))?)
    }

    fn optional<T>(
        &self,
        name: &str,
        convert: impl FnOnce(&str, &'a Value) -> Result<T, Error>,
    ) -> Result<Option<T>, Error> {
        self.get(name).map(|value| convert(name, value)).transpose()
    }
}

fn invalid_request(message: impl Into<String>) -> Error {
    Error::new(ErrorCode::InvalidRequest, message)
}

fn wrong_type(name: &str, expected: &str) -> Error {
    Error::invalid_value(format!("{name} must be {expected}"))
}

fn string<'a>(name: &str, value: &'a Value) -> Result<&'a str, Error> {
    value.as_str().ok_or_else(|| wrong_type(name, "a string"))
}

fn boolean(name: &str, value: &Value) -> Result<bool, Error> {
    value.as_bool().ok_or_else(|| wrong_type(name, "a boolean"))
}

/// With `arbitrary_precision`, `as_i64` parses the number's own text, so it is `None` for a
/// fraction or an exponent (`30.0`, `3e1`) and still accepts `-0`.
fn integer(name: &str, value: &Value, range: RangeInclusive<u64>) -> Result<u64, Error> {
    value
        .as_i64()
        .and_then(|n| u64::try_from(n).ok())
        .filter(|n| range.contains(n))
        .ok_or_else(|| {
            Error::invalid_value(format!(
                "{name} must be an integer from {} to {}",
                range.start(),
                range.end()
            ))
        })
}

/// A group or deduplication id: 1 to 128 characters of `A-Z a-z 0-9 _ -`.
fn identifier<'a>(name: &str, value: &'a Value) -> Result<&'a str, Error> {
    let id = string(name, value)?;
    if IDENTIFIER_LEN.contains(&id.len()) && id.bytes().all(is_name_byte) {
        Ok(id)
    } else {
        Err(Error::invalid_value(format!(
            "{name} must be 1 to 128 characters of A-Z a-z 0-9 _ -"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code(line: &str) -> ErrorCode {
        parse(line.as_bytes()).unwrap_err().code
    }

    #[test]
    fn rejects_lines_that_are_not_a_command_object() {
        for line in [
            "",
            "   ",
            "not json",
            "[]",
            "\"Tick\"",
            "{}",
            r#"{"Command": 5}"#,
            r#"{"Command": null}"#,
            r#"{"Command": "tick", "Seconds": 1}"#,
            r#"{"Command": "Nope"}"#,
            r#"{"Command": "Tick", "Seconds": 1} trailing"#,
        ] {
            assert_eq!(code(line), ErrorCode::InvalidRequest, "line {line:?}");
        }
    }

    #[test]
    fn rejects_invalid_utf8() {
        let err = parse(b"{\"Command\": \"Tick\", \"Seconds\": 1, \"X\": \"\xff\"}").unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidRequest);
    }

    #[test]
    fn integers_must_be_written_without_fraction_or_exponent() {
        for text in [
            "30.0",
            "3e1",
            "\"30\"",
            "true",
            "-1",
            "1209601",
            "18446744073709551616",
        ] {
            let line = format!(r#"{{"Command": "Tick", "Seconds": {text}}}"#);
            assert_eq!(
                code(&line),
                ErrorCode::InvalidParameterValue,
                "Seconds {text}"
            );
        }
        for (text, want) in [("0", 0), ("-0", 0), ("30", 30), ("1209600", 1_209_600)] {
            let line = format!(r#"{{"Command": "Tick", "Seconds": {text}}}"#);
            assert_eq!(parse(line.as_bytes()), Ok(Command::Tick { seconds: want }));
        }
    }

    #[test]
    fn null_and_unknown_members_are_ignored() {
        let line =
            r#"{"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": null, "Extra": 1}"#;
        assert_eq!(
            parse(line.as_bytes()),
            Ok(Command::ReceiveMessage {
                queue: "q".into(),
                max_messages: 1,
                visibility_timeout: None,
            })
        );
        assert_eq!(
            code(r#"{"Command": "PurgeQueue", "QueueName": null}"#),
            ErrorCode::MissingParameter
        );
    }

    #[test]
    fn missing_parameters_are_reported_before_bad_values() {
        let line = r#"{"Command": "SendMessage", "QueueName": 5}"#;
        assert_eq!(code(line), ErrorCode::MissingParameter);
        let line =
            r#"{"Command": "ChangeMessageVisibility", "QueueName": "q", "ReceiptHandle": 5}"#;
        assert_eq!(code(line), ErrorCode::MissingParameter);
    }

    #[test]
    fn queue_name_syntax_is_checked_first_in_create_queue() {
        let long = "a".repeat(81);
        let long_fifo = format!("{}.fifo", "a".repeat(76));
        for name in [
            "",
            "a b",
            "a.b",
            ".fifo",
            "é",
            long.as_str(),
            long_fifo.as_str(),
        ] {
            let line = serde_json::json!({
                "Command": "CreateQueue",
                "QueueName": name,
                "Attributes": 5,
            })
            .to_string();
            assert_eq!(code(&line), ErrorCode::QueueNameInvalid, "name {name:?}");
        }
        let line = r#"{"Command": "CreateQueue", "QueueName": 5}"#;
        assert_eq!(code(line), ErrorCode::InvalidParameterValue);
        let line = r#"{"Command": "CreateQueue", "QueueName": "q", "Attributes": 5}"#;
        assert_eq!(code(line), ErrorCode::InvalidParameterValue);
    }

    #[test]
    fn accepts_names_at_the_length_limits() {
        for name in [
            "a".repeat(80),
            format!("{}.fifo", "a".repeat(75)),
            "A-z_9".into(),
        ] {
            let line = serde_json::json!({"Command": "CreateQueue", "QueueName": name}).to_string();
            assert!(parse(line.as_bytes()).is_ok(), "name {name:?}");
        }
    }

    #[test]
    fn attribute_values_are_validated_one_by_one() {
        for attributes in [
            r#"{"VisibilityTimeout": 43201}"#,
            r#"{"DelaySeconds": 901}"#,
            r#"{"RetentionSeconds": 59}"#,
            r#"{"MaxReceiveCount": 0}"#,
            r#"{"Fifo": "true"}"#,
            r#"{"ContentBasedDeduplication": 1}"#,
            r#"{"DeadLetterQueue": 1}"#,
            r#"{"Unknown": 1}"#,
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
    fn parses_attributes_and_treats_null_as_absent() {
        let line = r#"{"Command": "SetQueueAttributes", "QueueName": "q",
            "Attributes": {"VisibilityTimeout": 5, "DelaySeconds": null, "DeadLetterQueue": "d", "MaxReceiveCount": 3}}"#;
        let Ok(Command::SetQueueAttributes { attributes, .. }) = parse(line.as_bytes()) else {
            panic!("expected SetQueueAttributes");
        };
        assert_eq!(
            attributes,
            AttributeUpdate {
                visibility_timeout: Some(5),
                dead_letter_queue: Some("d".into()),
                max_receive_count: Some(3),
                ..AttributeUpdate::default()
            }
        );
        assert_eq!(
            attributes.dead_letter(),
            Ok(Some(DeadLetter {
                queue: "d".into(),
                max_receive_count: 3
            }))
        );
    }

    #[test]
    fn dead_letter_attributes_come_as_a_pair() {
        let half = AttributeUpdate {
            max_receive_count: Some(2),
            ..AttributeUpdate::default()
        };
        assert_eq!(
            half.dead_letter().unwrap_err().code,
            ErrorCode::InvalidParameterValue
        );
        assert_eq!(AttributeUpdate::default().dead_letter(), Ok(None));
    }

    #[test]
    fn send_message_checks_body_and_identifier_rules() {
        let at_limit = "x".repeat(1024);
        let over_limit = "x".repeat(1025);
        let ok = serde_json::json!({"Command": "SendMessage", "QueueName": "q", "Body": at_limit});
        assert!(parse(ok.to_string().as_bytes()).is_ok());
        for (member, value) in [
            ("Body", serde_json::json!("")),
            ("Body", serde_json::json!(over_limit)),
            ("Body", serde_json::json!(5)),
            ("GroupId", serde_json::json!("")),
            ("GroupId", serde_json::json!("a".repeat(129))),
            ("DeduplicationId", serde_json::json!("a b")),
            ("DelaySeconds", serde_json::json!(901)),
        ] {
            let mut line =
                serde_json::json!({"Command": "SendMessage", "QueueName": "q", "Body": "b"});
            line[member] = value.clone();
            assert_eq!(
                code(&line.to_string()),
                ErrorCode::InvalidParameterValue,
                "{member}={value}"
            );
        }
    }

    #[test]
    fn two_byte_characters_count_as_two_body_bytes() {
        let body = "é".repeat(513);
        let line = serde_json::json!({"Command": "SendMessage", "QueueName": "q", "Body": body});
        assert_eq!(code(&line.to_string()), ErrorCode::InvalidParameterValue);
    }
}
