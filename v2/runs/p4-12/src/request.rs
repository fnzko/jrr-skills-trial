//! Turning a request line into a typed [`Command`].
//!
//! This covers checks 1 to 3 of SPEC section 2: request shape, required parameters and parameter
//! values. Everything that needs the service state (queue lookup and the per-command rules) happens
//! later, in [`crate::service`].

use std::ops::RangeInclusive;

use serde_json::{Map, Value};

use crate::error::{Error, ErrorCode, Result};

pub const VISIBILITY_TIMEOUT_RANGE: RangeInclusive<u64> = 0..=43_200;
pub const DELAY_SECONDS_RANGE: RangeInclusive<u64> = 0..=900;
pub const RETENTION_SECONDS_RANGE: RangeInclusive<u64> = 60..=1_209_600;
pub const MAX_RECEIVE_COUNT_RANGE: RangeInclusive<u64> = 1..=1_000;
pub const MAX_MESSAGES_RANGE: RangeInclusive<u64> = 1..=10;
pub const TICK_SECONDS_RANGE: RangeInclusive<u64> = 0..=1_209_600;
pub const BODY_BYTES_RANGE: RangeInclusive<usize> = 1..=1024;
pub const MESSAGE_TOKEN_LEN_RANGE: RangeInclusive<usize> = 1..=128;

pub const FIFO_SUFFIX: &str = ".fifo";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
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
    ReceiveMessage {
        queue_name: String,
        max_messages: u64,
        visibility_timeout: Option<u64>,
    },
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendMessage {
    pub queue_name: String,
    pub body: String,
    pub delay_seconds: Option<u64>,
    pub group_id: Option<String>,
    pub deduplication_id: Option<String>,
}

/// The queue attributes named in a request, each already checked against its type and range.
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

impl AttributeChanges {
    /// Whether the request names exactly one half of the dead-letter configuration.
    pub fn has_partial_dead_letter_config(&self) -> bool {
        self.dead_letter_queue.is_some() != self.max_receive_count.is_some()
    }
}

/// Parses one request line into a command, or fails with the code of the first failing check.
pub fn parse(line: &[u8]) -> Result<Command> {
    let text = std::str::from_utf8(line)
        .map_err(|_| Error::invalid_request("the line is not valid UTF-8"))?;
    let Ok(Value::Object(members)) = serde_json::from_str::<Value>(text) else {
        return Err(Error::invalid_request("the line is not a JSON object"));
    };
    let params = Params(&members);
    let Some(Value::String(name)) = params.get("Command") else {
        return Err(Error::invalid_request("the request has no string Command"));
    };

    match name.as_str() {
        "CreateQueue" => {
            params.require(&["QueueName"])?;
            // QueueName is checked before every other parameter, with its own error code.
            let queue_name = params.string("QueueName")?;
            check_queue_name(&queue_name)?;
            Ok(Command::CreateQueue {
                queue_name,
                attributes: params.attributes()?.unwrap_or_default(),
            })
        }
        "SetQueueAttributes" => {
            params.require(&["QueueName", "Attributes"])?;
            Ok(Command::SetQueueAttributes {
                queue_name: params.string("QueueName")?,
                attributes: params.attributes()?.unwrap_or_default(),
            })
        }
        "GetQueueAttributes" => {
            params.require(&["QueueName"])?;
            Ok(Command::GetQueueAttributes {
                queue_name: params.string("QueueName")?,
            })
        }
        "ListQueues" => Ok(Command::ListQueues {
            prefix: params.optional_string("Prefix")?,
        }),
        "DeleteQueue" => {
            params.require(&["QueueName"])?;
            Ok(Command::DeleteQueue {
                queue_name: params.string("QueueName")?,
            })
        }
        "PurgeQueue" => {
            params.require(&["QueueName"])?;
            Ok(Command::PurgeQueue {
                queue_name: params.string("QueueName")?,
            })
        }
        "Tick" => {
            params.require(&["Seconds"])?;
            Ok(Command::Tick {
                seconds: params.integer("Seconds", TICK_SECONDS_RANGE)?,
            })
        }
        "SendMessage" => {
            params.require(&["QueueName", "Body"])?;
            let body = params.string("Body")?;
            if !BODY_BYTES_RANGE.contains(&body.len()) {
                return Err(Error::invalid_value("Body must be 1 to 1024 bytes long"));
            }
            Ok(Command::SendMessage(SendMessage {
                queue_name: params.string("QueueName")?,
                body,
                delay_seconds: params.optional_integer("DelaySeconds", DELAY_SECONDS_RANGE)?,
                group_id: params.optional_token("GroupId")?,
                deduplication_id: params.optional_token("DeduplicationId")?,
            }))
        }
        "ReceiveMessage" => {
            params.require(&["QueueName"])?;
            Ok(Command::ReceiveMessage {
                queue_name: params.string("QueueName")?,
                max_messages: params
                    .optional_integer("MaxMessages", MAX_MESSAGES_RANGE)?
                    .unwrap_or(1),
                visibility_timeout: params
                    .optional_integer("VisibilityTimeout", VISIBILITY_TIMEOUT_RANGE)?,
            })
        }
        "DeleteMessage" => {
            params.require(&["QueueName", "ReceiptHandle"])?;
            Ok(Command::DeleteMessage {
                queue_name: params.string("QueueName")?,
                receipt_handle: params.string("ReceiptHandle")?,
            })
        }
        "ChangeMessageVisibility" => {
            params.require(&["QueueName", "ReceiptHandle", "VisibilityTimeout"])?;
            Ok(Command::ChangeMessageVisibility {
                queue_name: params.string("QueueName")?,
                receipt_handle: params.string("ReceiptHandle")?,
                visibility_timeout: params
                    .integer("VisibilityTimeout", VISIBILITY_TIMEOUT_RANGE)?,
            })
        }
        other => Err(Error::invalid_request(format!("unknown command {other:?}"))),
    }
}

/// Checks the queue name syntax of SPEC section 4.
pub fn check_queue_name(name: &str) -> Result<()> {
    let base = name.strip_suffix(FIFO_SUFFIX).unwrap_or(name);
    let max_base_len = if base.len() == name.len() { 80 } else { 75 };
    if (1..=max_base_len).contains(&base.len()) && base.bytes().all(is_token_byte) {
        Ok(())
    } else {
        Err(Error::new(
            ErrorCode::QueueNameInvalid,
            format!("{name:?} is not a valid queue name"),
        ))
    }
}

/// Parses the text of a JSON number written without fraction or exponent, if it is not negative
/// and fits in a `u64`.
fn parse_integer(text: &str) -> Option<u64> {
    let digits = text.strip_prefix('-').unwrap_or(text);
    if !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let magnitude: u64 = digits.parse().ok()?;
    (magnitude == 0 || digits.len() == text.len()).then_some(magnitude)
}

/// The characters allowed in queue names, group ids and deduplication ids.
fn is_token_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-'
}

/// Typed access to the members of a request object (or of its `Attributes` object), where a `null`
/// member counts as absent.
struct Params<'a>(&'a Map<String, Value>);

impl Params<'_> {
    fn get(&self, name: &str) -> Option<&Value> {
        self.0.get(name).filter(|value| !value.is_null())
    }

    /// Fails with `MissingParameter` unless all `names` are present. Commands call this before
    /// looking at any value, because a missing parameter outranks a malformed one.
    fn require(&self, names: &[&str]) -> Result<()> {
        match names.iter().find(|name| self.get(name).is_none()) {
            Some(name) => Err(Error::missing_parameter(name)),
            None => Ok(()),
        }
    }

    fn string(&self, name: &str) -> Result<String> {
        self.optional_string(name)?
            .ok_or_else(|| Error::missing_parameter(name))
    }

    fn optional_string(&self, name: &str) -> Result<Option<String>> {
        match self.get(name) {
            None => Ok(None),
            Some(Value::String(s)) => Ok(Some(s.clone())),
            Some(_) => Err(Error::invalid_value(format!("{name} must be a string"))),
        }
    }

    fn integer(&self, name: &str, range: RangeInclusive<u64>) -> Result<u64> {
        self.optional_integer(name, range)?
            .ok_or_else(|| Error::missing_parameter(name))
    }

    /// Reads an integer in `range`. Whether a number is an integer depends on how it is written,
    /// so this looks at its source text (kept by serde_json's `arbitrary_precision` feature): `-0`
    /// is the integer 0, while `30.0` and `3e1` are not integers at all.
    fn optional_integer(&self, name: &str, range: RangeInclusive<u64>) -> Result<Option<u64>> {
        let Some(value) = self.get(name) else {
            return Ok(None);
        };
        let integer = match value {
            Value::Number(n) => parse_integer(&n.to_string()),
            _ => None,
        };
        integer
            .filter(|n| range.contains(n))
            .map(Some)
            .ok_or_else(|| {
                Error::invalid_value(format!(
                    "{name} must be an integer from {} to {}",
                    range.start(),
                    range.end()
                ))
            })
    }

    fn optional_bool(&self, name: &str) -> Result<Option<bool>> {
        match self.get(name) {
            None => Ok(None),
            Some(Value::Bool(b)) => Ok(Some(*b)),
            Some(_) => Err(Error::invalid_value(format!("{name} must be a boolean"))),
        }
    }

    /// Reads a group id or deduplication id: 1 to 128 characters from `A-Z a-z 0-9 _ -`.
    fn optional_token(&self, name: &str) -> Result<Option<String>> {
        let token = self.optional_string(name)?;
        match token {
            Some(t)
                if !(MESSAGE_TOKEN_LEN_RANGE.contains(&t.len())
                    && t.bytes().all(is_token_byte)) =>
            {
                Err(Error::invalid_value(format!(
                    "{name} must be 1 to 128 characters from A-Z a-z 0-9 _ -"
                )))
            }
            token => Ok(token),
        }
    }

    fn attributes(&self) -> Result<Option<AttributeChanges>> {
        let members = match self.get("Attributes") {
            None => return Ok(None),
            Some(Value::Object(members)) => members,
            Some(_) => return Err(Error::invalid_value("Attributes must be an object")),
        };
        const KNOWN: [&str; 7] = [
            "VisibilityTimeout",
            "DelaySeconds",
            "RetentionSeconds",
            "Fifo",
            "ContentBasedDeduplication",
            "DeadLetterQueue",
            "MaxReceiveCount",
        ];
        if let Some(unknown) = members.keys().find(|key| !KNOWN.contains(&key.as_str())) {
            return Err(Error::invalid_value(format!(
                "unknown attribute {unknown:?}"
            )));
        }
        let attrs = Params(members);
        Ok(Some(AttributeChanges {
            visibility_timeout: attrs
                .optional_integer("VisibilityTimeout", VISIBILITY_TIMEOUT_RANGE)?,
            delay_seconds: attrs.optional_integer("DelaySeconds", DELAY_SECONDS_RANGE)?,
            retention_seconds: attrs
                .optional_integer("RetentionSeconds", RETENTION_SECONDS_RANGE)?,
            fifo: attrs.optional_bool("Fifo")?,
            content_based_deduplication: attrs.optional_bool("ContentBasedDeduplication")?,
            dead_letter_queue: attrs.optional_string("DeadLetterQueue")?,
            max_receive_count: attrs
                .optional_integer("MaxReceiveCount", MAX_RECEIVE_COUNT_RANGE)?,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code_of(line: &str) -> ErrorCode {
        parse(line.as_bytes()).expect_err("parse should fail").code
    }

    #[test]
    fn rejects_lines_that_are_not_json_objects() {
        for line in ["", "   ", "[]", "42", "\"CreateQueue\"", "{", "{} {}"] {
            assert_eq!(code_of(line), ErrorCode::InvalidRequest, "line {line:?}");
        }
        assert_eq!(
            parse(b"{\"Command\": \"Tick\", \"Seconds\": 1, \"x\": \"\xff\"}")
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }

    #[test]
    fn requires_a_known_string_command() {
        assert_eq!(code_of("{}"), ErrorCode::InvalidRequest);
        assert_eq!(code_of(r#"{"Command": null}"#), ErrorCode::InvalidRequest);
        assert_eq!(code_of(r#"{"Command": 1}"#), ErrorCode::InvalidRequest);
        assert_eq!(code_of(r#"{"Command": "tick"}"#), ErrorCode::InvalidRequest);
    }

    #[test]
    fn integers_must_be_written_without_fraction_or_exponent() {
        let ok = parse(br#"{"Command": "Tick", "Seconds": 30}"#).unwrap();
        assert_eq!(ok, Command::Tick { seconds: 30 });
        let zero = parse(br#"{"Command": "Tick", "Seconds": -0}"#).unwrap();
        assert_eq!(zero, Command::Tick { seconds: 0 });
        for value in [
            "30.0",
            "3e1",
            "3E1",
            "0.0",
            "\"30\"",
            "-1",
            "1209601",
            "18446744073709551616",
            "true",
        ] {
            let line = format!(r#"{{"Command": "Tick", "Seconds": {value}}}"#);
            assert_eq!(code_of(&line), ErrorCode::InvalidParameterValue, "{value}");
        }
    }

    #[test]
    fn null_counts_as_absent_and_unknown_members_are_ignored() {
        assert_eq!(
            code_of(r#"{"Command": "Tick", "Seconds": null}"#),
            ErrorCode::MissingParameter
        );
        let cmd = parse(br#"{"Command": "ListQueues", "Prefix": null, "Extra": [1]}"#).unwrap();
        assert_eq!(cmd, Command::ListQueues { prefix: None });
    }

    #[test]
    fn missing_parameters_outrank_invalid_ones() {
        assert_eq!(
            code_of(r#"{"Command": "SendMessage", "QueueName": 7}"#),
            ErrorCode::MissingParameter
        );
        assert_eq!(
            code_of(
                r#"{"Command": "ChangeMessageVisibility", "QueueName": "q", "ReceiptHandle": 1}"#
            ),
            ErrorCode::MissingParameter
        );
    }

    #[test]
    fn create_queue_checks_the_name_before_attributes() {
        assert_eq!(
            code_of(r#"{"Command": "CreateQueue", "QueueName": "bad name", "Attributes": 5}"#),
            ErrorCode::QueueNameInvalid
        );
        assert_eq!(
            code_of(r#"{"Command": "CreateQueue", "QueueName": 5}"#),
            ErrorCode::InvalidParameterValue
        );
        assert_eq!(
            code_of(r#"{"Command": "CreateQueue", "QueueName": "q", "Attributes": {"Color": 1}}"#),
            ErrorCode::InvalidParameterValue
        );
    }

    #[test]
    fn other_commands_accept_any_queue_name_string() {
        let cmd = parse(br#"{"Command": "PurgeQueue", "QueueName": "not a name!"}"#).unwrap();
        assert_eq!(
            cmd,
            Command::PurgeQueue {
                queue_name: "not a name!".into()
            }
        );
    }

    #[test]
    fn queue_name_syntax() {
        let valid = [
            "a",
            "A-z_09",
            &"x".repeat(80),
            "a.fifo",
            &format!("{}.fifo", "x".repeat(75)),
        ];
        for name in valid {
            assert!(check_queue_name(name).is_ok(), "{name:?} should be valid");
        }
        let long_fifo = format!("{}.fifo", "x".repeat(76));
        let invalid = [
            "",
            ".fifo",
            "a.b",
            "a b",
            "é",
            &"x".repeat(81),
            &long_fifo,
            "a.fifo.fifo",
            "a.FIFO",
        ];
        for name in invalid {
            assert!(
                check_queue_name(name).is_err(),
                "{name:?} should be invalid"
            );
        }
    }

    #[test]
    fn body_length_is_measured_in_utf8_bytes() {
        let body = "é".repeat(512);
        let line = serde_json::json!({"Command": "SendMessage", "QueueName": "q", "Body": body});
        assert!(parse(line.to_string().as_bytes()).is_ok());
        let body = "é".repeat(513);
        let line = serde_json::json!({"Command": "SendMessage", "QueueName": "q", "Body": body});
        assert_eq!(code_of(&line.to_string()), ErrorCode::InvalidParameterValue);
        assert_eq!(
            code_of(r#"{"Command": "SendMessage", "QueueName": "q", "Body": ""}"#),
            ErrorCode::InvalidParameterValue
        );
    }

    #[test]
    fn group_and_deduplication_ids_are_restricted_tokens() {
        let ok = format!(
            r#"{{"Command": "SendMessage", "QueueName": "q", "Body": "b", "GroupId": "{}"}}"#,
            "g".repeat(128)
        );
        assert!(parse(ok.as_bytes()).is_ok());
        for id in ["", "a.b", &"g".repeat(129)] {
            let line = format!(
                r#"{{"Command": "SendMessage", "QueueName": "q", "Body": "b", "DeduplicationId": "{id}"}}"#
            );
            assert_eq!(code_of(&line), ErrorCode::InvalidParameterValue, "{id:?}");
        }
    }
}
