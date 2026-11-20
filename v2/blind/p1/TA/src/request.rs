//! Turns an input line into a typed [`Request`].
//!
//! This is the whole of the "request shape", "required parameters" and
//! "parameter values" checks of the spec (steps 1 to 3 of its error order),
//! so everything past this module can assume well-typed, in-range values.

use std::ops::RangeInclusive;

use serde_json::{Map, Value};

use crate::config::{AttributePatch, DeadLetter};
use crate::error::{Error, ErrorCode};
use crate::ids::{DeduplicationId, GroupId, is_name_char};

const MAX_BODY_BYTES: usize = 1024;
const MAX_QUEUE_NAME_CHARS: usize = 80;
const FIFO_SUFFIX: &str = ".fifo";

#[derive(Debug, PartialEq, Eq)]
pub enum Request {
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
    pub group_id: Option<GroupId>,
    pub deduplication_id: Option<DeduplicationId>,
}

impl AttributePatch {
    /// The dead-letter configuration the patch asks for, if any. Naming only
    /// one of its two halves is an error.
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

/// Parses one input line, without its line terminator.
pub fn parse(line: &[u8]) -> Result<Request, Error> {
    let text = std::str::from_utf8(line)
        .map_err(|e| Error::invalid_request(format!("line is not valid UTF-8: {e}")))?;
    let value: Value = serde_json::from_str(text)
        .map_err(|e| Error::invalid_request(format!("line is not valid JSON: {e}")))?;
    let Value::Object(members) = value else {
        return Err(Error::invalid_request("line is not a JSON object"));
    };
    let params = Params(&members);
    let Some(Value::String(command)) = params.get("Command") else {
        return Err(Error::invalid_request("Command must be a string"));
    };

    match command.as_str() {
        "CreateQueue" => {
            let name = params.require("QueueName")?;
            // The name is checked before every other parameter.
            let name = queue_name(name)?;
            let attributes = params
                .optional("Attributes", attributes)?
                .unwrap_or_default();
            Ok(Request::CreateQueue { name, attributes })
        }
        "SetQueueAttributes" => {
            let queue = params.require("QueueName")?;
            let attributes_value = params.require("Attributes")?;
            Ok(Request::SetQueueAttributes {
                queue: string("QueueName", queue)?.to_owned(),
                attributes: attributes("Attributes", attributes_value)?,
            })
        }
        "GetQueueAttributes" => Ok(Request::GetQueueAttributes {
            queue: params.queue()?,
        }),
        "ListQueues" => Ok(Request::ListQueues {
            prefix: params.optional("Prefix", string)?.map(str::to_owned),
        }),
        "DeleteQueue" => Ok(Request::DeleteQueue {
            queue: params.queue()?,
        }),
        "PurgeQueue" => Ok(Request::PurgeQueue {
            queue: params.queue()?,
        }),
        "Tick" => {
            let seconds = params.require("Seconds")?;
            Ok(Request::Tick {
                seconds: integer("Seconds", seconds, 0..=1_209_600)?,
            })
        }
        "SendMessage" => {
            let queue = params.require("QueueName")?;
            let body = params.require("Body")?;
            Ok(Request::SendMessage(SendMessage {
                queue: string("QueueName", queue)?.to_owned(),
                body: message_body(body)?,
                delay_seconds: params.optional("DelaySeconds", |n, v| integer(n, v, 0..=900))?,
                group_id: params.optional("GroupId", |n, v| GroupId::parse(string(n, v)?))?,
                deduplication_id: params.optional("DeduplicationId", |n, v| {
                    DeduplicationId::parse(string(n, v)?)
                })?,
            }))
        }
        "ReceiveMessage" => {
            let queue = params.require("QueueName")?;
            Ok(Request::ReceiveMessage {
                queue: string("QueueName", queue)?.to_owned(),
                max_messages: params
                    .optional("MaxMessages", |n, v| integer(n, v, 1..=10))?
                    .unwrap_or(1),
                visibility_timeout: params
                    .optional("VisibilityTimeout", |n, v| integer(n, v, 0..=43_200))?,
            })
        }
        "DeleteMessage" => {
            let queue = params.require("QueueName")?;
            let receipt_handle = params.require("ReceiptHandle")?;
            Ok(Request::DeleteMessage {
                queue: string("QueueName", queue)?.to_owned(),
                receipt_handle: string("ReceiptHandle", receipt_handle)?.to_owned(),
            })
        }
        "ChangeMessageVisibility" => {
            let queue = params.require("QueueName")?;
            let receipt_handle = params.require("ReceiptHandle")?;
            let timeout = params.require("VisibilityTimeout")?;
            Ok(Request::ChangeMessageVisibility {
                queue: string("QueueName", queue)?.to_owned(),
                receipt_handle: string("ReceiptHandle", receipt_handle)?.to_owned(),
                visibility_timeout: integer("VisibilityTimeout", timeout, 0..=43_200)?,
            })
        }
        other => Err(Error::invalid_request(format!("unknown command {other:?}"))),
    }
}

/// The members of a command object. A `null` member counts as absent.
struct Params<'a>(&'a Map<String, Value>);

impl<'a> Params<'a> {
    fn get(&self, name: &str) -> Option<&'a Value> {
        self.0.get(name).filter(|value| !value.is_null())
    }

    fn require(&self, name: &str) -> Result<&'a Value, Error> {
        self.get(name).ok_or_else(|| Error::missing(name))
    }

    fn optional<T>(
        &self,
        name: &'static str,
        parse: impl FnOnce(&'static str, &'a Value) -> Result<T, Error>,
    ) -> Result<Option<T>, Error> {
        self.get(name).map(|value| parse(name, value)).transpose()
    }

    /// The `QueueName` of a command that takes nothing else and accepts any
    /// string there (only `CreateQueue` restricts names).
    fn queue(&self) -> Result<String, Error> {
        string("QueueName", self.require("QueueName")?).map(str::to_owned)
    }
}

fn string<'a>(name: &str, value: &'a Value) -> Result<&'a str, Error> {
    value
        .as_str()
        .ok_or_else(|| Error::invalid_value(format!("{name} must be a string")))
}

fn boolean(name: &str, value: &Value) -> Result<bool, Error> {
    value
        .as_bool()
        .ok_or_else(|| Error::invalid_value(format!("{name} must be a boolean")))
}

/// An integer written without fraction or exponent, within `range`.
fn integer(name: &str, value: &Value, range: RangeInclusive<u64>) -> Result<u64, Error> {
    let Some(number) = value.as_i64() else {
        return Err(Error::invalid_value(format!("{name} must be an integer")));
    };
    u64::try_from(number)
        .ok()
        .filter(|n| range.contains(n))
        .ok_or_else(|| {
            Error::invalid_value(format!(
                "{name} must be between {} and {}",
                range.start(),
                range.end()
            ))
        })
}

fn message_body(value: &Value) -> Result<String, Error> {
    let body = string("Body", value)?;
    if body.is_empty() || body.len() > MAX_BODY_BYTES {
        return Err(Error::invalid_value(format!(
            "Body must be 1 to {MAX_BODY_BYTES} bytes"
        )));
    }
    Ok(body.to_owned())
}

/// The `QueueName` of `CreateQueue`: any string that is not a legal name is
/// `QueueNameInvalid`, a non-string is `InvalidParameterValue`.
fn queue_name(value: &Value) -> Result<String, Error> {
    let name = string("QueueName", value)?;
    let (stem, max) = match name.strip_suffix(FIFO_SUFFIX) {
        Some(stem) => (stem, MAX_QUEUE_NAME_CHARS - FIFO_SUFFIX.len()),
        None => (name, MAX_QUEUE_NAME_CHARS),
    };
    if stem.is_empty() || stem.len() > max || !stem.bytes().all(is_name_char) {
        return Err(Error::new(
            ErrorCode::QueueNameInvalid,
            format!(
                "queue name {name:?} must be 1 to {MAX_QUEUE_NAME_CHARS} characters from \
                 A-Z a-z 0-9 _ -, optionally followed by {FIFO_SUFFIX}"
            ),
        ));
    }
    Ok(name.to_owned())
}

fn attributes(name: &str, value: &Value) -> Result<AttributePatch, Error> {
    let Value::Object(members) = value else {
        return Err(Error::invalid_value(format!("{name} must be an object")));
    };
    let mut patch = AttributePatch::default();
    for (attribute, value) in members.iter().filter(|(_, value)| !value.is_null()) {
        match attribute.as_str() {
            "VisibilityTimeout" => {
                patch.visibility_timeout = Some(integer(attribute, value, 0..=43_200)?)
            }
            "DelaySeconds" => patch.delay_seconds = Some(integer(attribute, value, 0..=900)?),
            "RetentionSeconds" => {
                patch.retention_seconds = Some(integer(attribute, value, 60..=1_209_600)?)
            }
            "Fifo" => patch.fifo = Some(boolean(attribute, value)?),
            "ContentBasedDeduplication" => {
                patch.content_based_deduplication = Some(boolean(attribute, value)?)
            }
            "DeadLetterQueue" => {
                patch.dead_letter_queue = Some(string(attribute, value)?.to_owned())
            }
            "MaxReceiveCount" => {
                patch.max_receive_count = Some(integer(attribute, value, 1..=1000)?)
            }
            unknown => {
                return Err(Error::invalid_value(format!(
                    "unknown attribute {unknown:?}"
                )));
            }
        }
    }
    Ok(patch)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code(line: &str) -> ErrorCode {
        parse(line.as_bytes()).expect_err(line).code()
    }

    #[test]
    fn parses_a_simple_command() {
        assert_eq!(
            parse(br#"{"Command": "PurgeQueue", "QueueName": "jobs", "Extra": [1]}"#),
            Ok(Request::PurgeQueue {
                queue: "jobs".into()
            })
        );
    }

    #[test]
    fn rejects_lines_that_are_not_commands() {
        for line in [
            "",
            "   ",
            "[]",
            "42",
            "{",
            r#"{"Command": "Tick"} trailing"#,
            r#"{"command": "Tick", "Seconds": 1}"#,
            r#"{"Command": 7}"#,
            r#"{"Command": null}"#,
            r#"{"Command": "Nope"}"#,
        ] {
            assert_eq!(code(line), ErrorCode::InvalidRequest, "{line:?}");
        }
        assert_eq!(
            parse(b"{\"Command\": \"Tick\", \"Seconds\": 1, \"x\": \"\xff\"}")
                .unwrap_err()
                .code(),
            ErrorCode::InvalidRequest
        );
    }

    #[test]
    fn missing_parameters_win_over_invalid_values() {
        // Seconds is absent although QueueName would be invalid on its own.
        assert_eq!(
            code(
                r#"{"Command": "ChangeMessageVisibility", "QueueName": 5, "ReceiptHandle": "r-1"}"#
            ),
            ErrorCode::MissingParameter
        );
        assert_eq!(
            code(r#"{"Command": "Tick", "Seconds": null}"#),
            ErrorCode::MissingParameter
        );
    }

    #[test]
    fn integers_must_be_written_as_integers_in_range() {
        for seconds in [
            "1.0", "1e0", "\"1\"", "-1", "1209601", "true", "[1]", "1e400",
        ] {
            let line = format!(r#"{{"Command": "Tick", "Seconds": {seconds}}}"#);
            assert_eq!(code(&line), ErrorCode::InvalidParameterValue, "{seconds}");
        }
        for (seconds, expected) in [("0", 0), ("-0", 0), ("1209600", 1_209_600)] {
            let line = format!(r#"{{"Command": "Tick", "Seconds": {seconds}}}"#);
            assert_eq!(
                parse(line.as_bytes()),
                Ok(Request::Tick { seconds: expected })
            );
        }
    }

    #[test]
    fn create_queue_checks_the_name_before_other_parameters() {
        assert_eq!(
            code(r#"{"Command": "CreateQueue", "QueueName": "a b", "Attributes": 3}"#),
            ErrorCode::QueueNameInvalid
        );
        assert_eq!(
            code(r#"{"Command": "CreateQueue", "QueueName": 3, "Attributes": 3}"#),
            ErrorCode::InvalidParameterValue
        );
        assert_eq!(
            code(r#"{"Command": "CreateQueue", "QueueName": "ok", "Attributes": 3}"#),
            ErrorCode::InvalidParameterValue
        );
    }

    #[test]
    fn queue_name_syntax() {
        let long = "a".repeat(80);
        let long_fifo = format!("{}.fifo", "a".repeat(75));
        let too_long_fifo = format!("{}.fifo", "a".repeat(76));
        let too_long = "a".repeat(81);
        for ok in ["a", "A_z-9", "q.fifo", long.as_str(), long_fifo.as_str()] {
            assert_eq!(queue_name(&Value::from(ok)), Ok(ok.to_owned()), "{ok}");
        }
        for bad in [
            "",
            ".fifo",
            "a.b",
            "a.fifo.fifo",
            "a b",
            "é",
            "a.FIFO",
            too_long.as_str(),
            too_long_fifo.as_str(),
        ] {
            assert_eq!(
                queue_name(&Value::from(bad)).unwrap_err().code(),
                ErrorCode::QueueNameInvalid,
                "{bad:?}"
            );
        }
    }

    #[test]
    fn null_attributes_are_absent_even_when_unknown() {
        let request = parse(
            br#"{"Command": "CreateQueue", "QueueName": "q", "Attributes": {"Bogus": null, "DelaySeconds": null, "VisibilityTimeout": 5}}"#,
        );
        assert_eq!(
            request,
            Ok(Request::CreateQueue {
                name: "q".into(),
                attributes: AttributePatch {
                    visibility_timeout: Some(5),
                    ..AttributePatch::default()
                },
            })
        );
    }

    #[test]
    fn attribute_values_are_checked() {
        for attributes in [
            r#"{"Unknown": 1}"#,
            r#"{"VisibilityTimeout": 43201}"#,
            r#"{"DelaySeconds": 901}"#,
            r#"{"RetentionSeconds": 59}"#,
            r#"{"RetentionSeconds": 1209601}"#,
            r#"{"Fifo": "true"}"#,
            r#"{"ContentBasedDeduplication": 1}"#,
            r#"{"DeadLetterQueue": 1}"#,
            r#"{"MaxReceiveCount": 0}"#,
            r#"{"MaxReceiveCount": 1001}"#,
            r#"[]"#,
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
    fn send_message_parameter_rules() {
        let send = |extra: &str| {
            code(&format!(
                r#"{{"Command": "SendMessage", "QueueName": "q", "Body": "x"{extra}}}"#
            ))
        };
        assert_eq!(send(r#", "GroupId": """#), ErrorCode::InvalidParameterValue);
        assert_eq!(
            send(r#", "GroupId": "a b""#),
            ErrorCode::InvalidParameterValue
        );
        assert_eq!(
            send(r#", "DeduplicationId": 5"#),
            ErrorCode::InvalidParameterValue
        );
        assert_eq!(
            send(r#", "DelaySeconds": 901"#),
            ErrorCode::InvalidParameterValue
        );
        let long = "a".repeat(129);
        assert_eq!(
            send(&format!(r#", "GroupId": "{long}""#)),
            ErrorCode::InvalidParameterValue
        );
        assert_eq!(
            code(r#"{"Command": "SendMessage", "QueueName": "q", "Body": ""}"#),
            ErrorCode::InvalidParameterValue
        );
        let big = "é".repeat(513);
        assert_eq!(
            code(&format!(
                r#"{{"Command": "SendMessage", "QueueName": "q", "Body": "{big}"}}"#
            )),
            ErrorCode::InvalidParameterValue
        );
        let edge = "é".repeat(512);
        assert!(
            parse(
                format!(r#"{{"Command": "SendMessage", "QueueName": "q", "Body": "{edge}"}}"#)
                    .as_bytes()
            )
            .is_ok()
        );
    }

    #[test]
    fn receive_message_defaults() {
        assert_eq!(
            parse(br#"{"Command": "ReceiveMessage", "QueueName": "q"}"#),
            Ok(Request::ReceiveMessage {
                queue: "q".into(),
                max_messages: 1,
                visibility_timeout: None,
            })
        );
        assert_eq!(
            code(r#"{"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 11}"#),
            ErrorCode::InvalidParameterValue
        );
    }

    #[test]
    fn dead_letter_halves_must_come_together() {
        let half = AttributePatch {
            max_receive_count: Some(2),
            ..AttributePatch::default()
        };
        assert_eq!(
            half.dead_letter().unwrap_err().code(),
            ErrorCode::InvalidParameterValue
        );
        assert_eq!(AttributePatch::default().dead_letter(), Ok(None));
    }
}
