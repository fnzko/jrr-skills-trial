//! Turns one input line into a typed [`Request`].
//!
//! This module performs the first three checks of the error order: request
//! shape, required parameters, then each parameter value. Queue lookups and
//! command rules belong to the broker.

use std::ops::RangeInclusive;

use serde_json::{Map, Value};

use crate::error::{CommandError, ErrorCode};
use crate::types::{GroupId, QueueName, is_name_byte, is_token};

pub const VISIBILITY_TIMEOUT: RangeInclusive<u64> = 0..=43_200;
pub const DELAY_SECONDS: RangeInclusive<u64> = 0..=900;
pub const RETENTION_SECONDS: RangeInclusive<u64> = 60..=1_209_600;
pub const MAX_RECEIVE_COUNT: RangeInclusive<u64> = 1..=1000;
pub const TICK_SECONDS: RangeInclusive<u64> = 0..=1_209_600;
pub const MAX_MESSAGES: RangeInclusive<u64> = 1..=10;
pub const BODY_BYTES: RangeInclusive<usize> = 1..=1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    CreateQueue {
        queue: QueueName,
        attributes: Attributes,
    },
    SetQueueAttributes {
        queue: QueueName,
        attributes: Attributes,
    },
    GetQueueAttributes {
        queue: QueueName,
    },
    ListQueues {
        prefix: Option<String>,
    },
    DeleteQueue {
        queue: QueueName,
    },
    PurgeQueue {
        queue: QueueName,
    },
    Tick {
        seconds: u64,
    },
    SendMessage(SendMessage),
    ReceiveMessage {
        queue: QueueName,
        max_messages: usize,
        visibility_timeout: Option<u64>,
    },
    DeleteMessage {
        queue: QueueName,
        receipt_handle: String,
    },
    ChangeMessageVisibility {
        queue: QueueName,
        receipt_handle: String,
        visibility_timeout: u64,
    },
}

/// The queue attributes a request names, each already within its range.
///
/// `None` means the request left the attribute out.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Attributes {
    pub visibility_timeout: Option<u64>,
    pub delay_seconds: Option<u64>,
    pub retention_seconds: Option<u64>,
    pub fifo: Option<bool>,
    pub content_based_deduplication: Option<bool>,
    pub dead_letter_queue: Option<QueueName>,
    pub max_receive_count: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendMessage {
    pub queue: QueueName,
    pub body: String,
    pub delay_seconds: Option<u64>,
    pub group_id: Option<GroupId>,
    /// Already checked against the token rule.
    pub deduplication_id: Option<String>,
}

/// Parses one line, without its trailing `\n`.
pub fn parse(line: &[u8]) -> Result<Request, CommandError> {
    // serde_json rejects invalid UTF-8 itself, so one error covers both cases.
    let value: Value = serde_json::from_slice(line).map_err(|e| {
        CommandError::new(ErrorCode::InvalidRequest, format!("not a JSON object: {e}"))
    })?;
    let Value::Object(members) = value else {
        return Err(CommandError::new(
            ErrorCode::InvalidRequest,
            "the line is not a JSON object",
        ));
    };
    let Some(Value::String(command)) = members.get("Command") else {
        return Err(CommandError::new(
            ErrorCode::InvalidRequest,
            "the request has no string member Command",
        ));
    };
    let params = Params(&members);
    match command.as_str() {
        "CreateQueue" => create_queue(params),
        "SetQueueAttributes" => {
            let queue = params.required("QueueName")?;
            let attributes = params.required("Attributes")?;
            Ok(Request::SetQueueAttributes {
                queue: queue_name("QueueName", queue)?,
                attributes: parse_attributes(attributes)?,
            })
        }
        "GetQueueAttributes" => Ok(Request::GetQueueAttributes {
            queue: params.queue()?,
        }),
        "ListQueues" => Ok(Request::ListQueues {
            prefix: params
                .optional("Prefix")
                .map(|v| string("Prefix", v).map(str::to_owned))
                .transpose()?,
        }),
        "DeleteQueue" => Ok(Request::DeleteQueue {
            queue: params.queue()?,
        }),
        "PurgeQueue" => Ok(Request::PurgeQueue {
            queue: params.queue()?,
        }),
        "Tick" => {
            let seconds = params.required("Seconds")?;
            Ok(Request::Tick {
                seconds: integer("Seconds", seconds, TICK_SECONDS)?,
            })
        }
        "SendMessage" => send_message(params),
        "ReceiveMessage" => receive_message(params),
        "DeleteMessage" => {
            let queue = params.required("QueueName")?;
            let receipt_handle = params.required("ReceiptHandle")?;
            Ok(Request::DeleteMessage {
                queue: queue_name("QueueName", queue)?,
                receipt_handle: string("ReceiptHandle", receipt_handle)?.to_owned(),
            })
        }
        "ChangeMessageVisibility" => {
            let queue = params.required("QueueName")?;
            let receipt_handle = params.required("ReceiptHandle")?;
            let visibility_timeout = params.required("VisibilityTimeout")?;
            Ok(Request::ChangeMessageVisibility {
                queue: queue_name("QueueName", queue)?,
                receipt_handle: string("ReceiptHandle", receipt_handle)?.to_owned(),
                visibility_timeout: integer(
                    "VisibilityTimeout",
                    visibility_timeout,
                    VISIBILITY_TIMEOUT,
                )?,
            })
        }
        other => Err(CommandError::new(
            ErrorCode::InvalidRequest,
            format!("unknown command {other:?}"),
        )),
    }
}

fn create_queue(params: Params<'_>) -> Result<Request, CommandError> {
    let queue = params.required("QueueName")?;
    // The name is checked before every other parameter, so its syntax error
    // wins over an invalid attribute.
    let queue = string("QueueName", queue)?;
    if !is_valid_queue_name(queue) {
        return Err(CommandError::new(
            ErrorCode::QueueNameInvalid,
            format!("{queue:?} breaks the queue name syntax"),
        ));
    }
    let attributes = params
        .optional("Attributes")
        .map(parse_attributes)
        .transpose()?
        .unwrap_or_default();
    Ok(Request::CreateQueue {
        queue: QueueName::new(queue),
        attributes,
    })
}

fn send_message(params: Params<'_>) -> Result<Request, CommandError> {
    let queue = params.required("QueueName")?;
    let body = params.required("Body")?;
    let queue = queue_name("QueueName", queue)?;
    let body = string("Body", body)?;
    if !BODY_BYTES.contains(&body.len()) {
        return Err(invalid_value(format!(
            "Body must be 1 to 1024 bytes long, got {}",
            body.len()
        )));
    }
    let delay_seconds = params
        .optional("DelaySeconds")
        .map(|v| integer("DelaySeconds", v, DELAY_SECONDS))
        .transpose()?;
    let group_id = params
        .optional("GroupId")
        .map(|v| {
            let s = string("GroupId", v)?;
            GroupId::parse(s).ok_or_else(|| token_error("GroupId"))
        })
        .transpose()?;
    let deduplication_id = params
        .optional("DeduplicationId")
        .map(|v| {
            let s = string("DeduplicationId", v)?;
            if is_token(s) {
                Ok(s.to_owned())
            } else {
                Err(token_error("DeduplicationId"))
            }
        })
        .transpose()?;
    Ok(Request::SendMessage(SendMessage {
        queue,
        body: body.to_owned(),
        delay_seconds,
        group_id,
        deduplication_id,
    }))
}

fn receive_message(params: Params<'_>) -> Result<Request, CommandError> {
    let queue = params.queue()?;
    let max_messages = match params.optional("MaxMessages") {
        Some(v) => integer("MaxMessages", v, MAX_MESSAGES)?,
        None => 1,
    };
    let visibility_timeout = params
        .optional("VisibilityTimeout")
        .map(|v| integer("VisibilityTimeout", v, VISIBILITY_TIMEOUT))
        .transpose()?;
    Ok(Request::ReceiveMessage {
        queue,
        max_messages: usize::try_from(max_messages).expect("MaxMessages is at most 10"),
        visibility_timeout,
    })
}

fn parse_attributes(value: &Value) -> Result<Attributes, CommandError> {
    let Value::Object(members) = value else {
        return Err(invalid_value("Attributes must be an object"));
    };
    let mut attributes = Attributes::default();
    for (name, value) in members {
        if value.is_null() {
            continue;
        }
        match name.as_str() {
            "VisibilityTimeout" => {
                attributes.visibility_timeout = Some(integer(name, value, VISIBILITY_TIMEOUT)?);
            }
            "DelaySeconds" => {
                attributes.delay_seconds = Some(integer(name, value, DELAY_SECONDS)?);
            }
            "RetentionSeconds" => {
                attributes.retention_seconds = Some(integer(name, value, RETENTION_SECONDS)?);
            }
            "Fifo" => attributes.fifo = Some(boolean(name, value)?),
            "ContentBasedDeduplication" => {
                attributes.content_based_deduplication = Some(boolean(name, value)?);
            }
            "DeadLetterQueue" => attributes.dead_letter_queue = Some(queue_name(name, value)?),
            "MaxReceiveCount" => {
                attributes.max_receive_count = Some(integer(name, value, MAX_RECEIVE_COUNT)?);
            }
            _ => return Err(invalid_value(format!("unknown attribute {name:?}"))),
        }
    }
    Ok(attributes)
}

/// 1 to 80 characters from `A-Z a-z 0-9 _ -`, or 1 to 75 of them followed by `.fifo`.
fn is_valid_queue_name(name: &str) -> bool {
    let (stem, max_len) = match name.strip_suffix(".fifo") {
        Some(stem) => (stem, 75),
        None => (name, 80),
    };
    // Every allowed character is one byte, so the byte length is the length
    // in characters whenever the character check passes.
    (1..=max_len).contains(&stem.len()) && stem.bytes().all(is_name_byte)
}

/// The members of a request object. A member whose value is `null` counts as absent.
#[derive(Clone, Copy)]
struct Params<'a>(&'a Map<String, Value>);

impl<'a> Params<'a> {
    fn optional(self, name: &str) -> Option<&'a Value> {
        self.0.get(name).filter(|v| !v.is_null())
    }

    fn required(self, name: &str) -> Result<&'a Value, CommandError> {
        self.optional(name).ok_or_else(|| {
            CommandError::new(
                ErrorCode::MissingParameter,
                format!("missing required parameter {name}"),
            )
        })
    }

    /// The required `QueueName` of a command whose only other parameters are optional.
    fn queue(self) -> Result<QueueName, CommandError> {
        queue_name("QueueName", self.required("QueueName")?)
    }
}

fn string<'v>(name: &str, value: &'v Value) -> Result<&'v str, CommandError> {
    value
        .as_str()
        .ok_or_else(|| invalid_value(format!("{name} must be a string")))
}

fn queue_name(name: &str, value: &Value) -> Result<QueueName, CommandError> {
    string(name, value).map(QueueName::new)
}

fn boolean(name: &str, value: &Value) -> Result<bool, CommandError> {
    value
        .as_bool()
        .ok_or_else(|| invalid_value(format!("{name} must be true or false")))
}

/// An integer within `range`. serde_json stores `30.0` and `3e1` as floats,
/// for which `as_u64` gives `None`, so only plain integer literals pass.
fn integer(name: &str, value: &Value, range: RangeInclusive<u64>) -> Result<u64, CommandError> {
    value.as_u64().filter(|n| range.contains(n)).ok_or_else(|| {
        invalid_value(format!(
            "{name} must be an integer from {} to {}",
            range.start(),
            range.end()
        ))
    })
}

fn token_error(name: &str) -> CommandError {
    invalid_value(format!(
        "{name} must be 1 to 128 characters from A-Z a-z 0-9 _ -"
    ))
}

fn invalid_value(message: impl Into<String>) -> CommandError {
    CommandError::new(ErrorCode::InvalidParameterValue, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code(line: &str) -> ErrorCode {
        parse(line.as_bytes()).unwrap_err().code
    }

    #[test]
    fn malformed_lines_are_invalid_requests() {
        let cases = [
            "",
            " ",
            "not json",
            "[]",
            "\"Tick\"",
            "{}",
            r#"{"Command": 1}"#,
            r#"{"Command": null}"#,
            r#"{"command": "Tick", "Seconds": 1}"#,
            r#"{"Command": "Nope"}"#,
            r#"{"Command": "tick", "Seconds": 1}"#,
            r#"{"Command": "Tick", "Seconds": 1} {}"#,
        ];
        for line in cases {
            assert_eq!(code(line), ErrorCode::InvalidRequest, "line {line:?}");
        }
    }

    #[test]
    fn invalid_utf8_is_an_invalid_request() {
        let line = b"{\"Command\": \"Tick\", \"Seconds\": 1, \"x\": \"\xff\"}";
        assert_eq!(parse(line).unwrap_err().code, ErrorCode::InvalidRequest);
    }

    #[test]
    fn integers_must_be_plain_literals_in_range() {
        let cases = [
            ("0", Ok(0)),
            ("30", Ok(30)),
            ("1209600", Ok(1_209_600)),
            ("1209601", Err(ErrorCode::InvalidParameterValue)),
            ("-1", Err(ErrorCode::InvalidParameterValue)),
            ("30.0", Err(ErrorCode::InvalidParameterValue)),
            ("3e1", Err(ErrorCode::InvalidParameterValue)),
            ("\"30\"", Err(ErrorCode::InvalidParameterValue)),
            ("true", Err(ErrorCode::InvalidParameterValue)),
            (
                "99999999999999999999999",
                Err(ErrorCode::InvalidParameterValue),
            ),
            ("null", Err(ErrorCode::MissingParameter)),
        ];
        for (seconds, want) in cases {
            let line = format!(r#"{{"Command": "Tick", "Seconds": {seconds}}}"#);
            let got = parse(line.as_bytes()).map_err(|e| e.code);
            assert_eq!(
                got,
                want.map(|seconds| Request::Tick { seconds }),
                "Seconds {seconds}"
            );
        }
    }

    #[test]
    fn missing_parameter_wins_over_invalid_value() {
        let line = r#"{"Command": "SendMessage", "QueueName": 5}"#;
        assert_eq!(code(line), ErrorCode::MissingParameter);
    }

    #[test]
    fn unknown_members_are_ignored() {
        let line = r#"{"Command": "PurgeQueue", "QueueName": "jobs", "Extra": [1]}"#;
        assert_eq!(
            parse(line.as_bytes()),
            Ok(Request::PurgeQueue {
                queue: QueueName::new("jobs")
            })
        );
    }

    #[test]
    fn queue_name_syntax() {
        let long_plain = "a".repeat(80);
        let too_long_plain = "a".repeat(81);
        let long_fifo = format!("{}.fifo", "a".repeat(75));
        let too_long_fifo = format!("{}.fifo", "a".repeat(76));
        let cases = [
            ("jobs", true),
            ("A-z_09", true),
            ("jobs.fifo", true),
            (long_plain.as_str(), true),
            (long_fifo.as_str(), true),
            ("", false),
            (".fifo", false),
            ("a.b", false),
            ("jobs.FIFO", false),
            ("a.fifo.fifo", false),
            ("jobs ", false),
            ("j\u{f6}bs", false),
            (too_long_plain.as_str(), false),
            (too_long_fifo.as_str(), false),
        ];
        for (name, valid) in cases {
            assert_eq!(is_valid_queue_name(name), valid, "name {name:?}");
        }
    }

    #[test]
    fn create_queue_checks_the_name_before_attributes() {
        let line = r#"{"Command": "CreateQueue", "QueueName": "a b", "Attributes": 3}"#;
        assert_eq!(code(line), ErrorCode::QueueNameInvalid);
        let line = r#"{"Command": "CreateQueue", "QueueName": 7, "Attributes": 3}"#;
        assert_eq!(code(line), ErrorCode::InvalidParameterValue);
    }

    #[test]
    fn attributes_are_parsed_and_null_means_absent() {
        let line = r#"{"Command": "CreateQueue", "QueueName": "q.fifo", "Attributes": {
            "VisibilityTimeout": 5, "DelaySeconds": 6, "RetentionSeconds": 60, "Fifo": true,
            "ContentBasedDeduplication": false, "DeadLetterQueue": "any thing",
            "MaxReceiveCount": 3, "Unknown": null}}"#;
        let want = Request::CreateQueue {
            queue: QueueName::new("q.fifo"),
            attributes: Attributes {
                visibility_timeout: Some(5),
                delay_seconds: Some(6),
                retention_seconds: Some(60),
                fifo: Some(true),
                content_based_deduplication: Some(false),
                dead_letter_queue: Some(QueueName::new("any thing")),
                max_receive_count: Some(3),
            },
        };
        assert_eq!(parse(line.as_bytes()), Ok(want));
    }

    #[test]
    fn bad_attributes_are_invalid_values() {
        let cases = [
            r#""x""#,
            r#"{"Unknown": 1}"#,
            r#"{"visibilityTimeout": 1}"#,
            r#"{"VisibilityTimeout": 43201}"#,
            r#"{"DelaySeconds": 901}"#,
            r#"{"RetentionSeconds": 59}"#,
            r#"{"Fifo": "true"}"#,
            r#"{"ContentBasedDeduplication": 1}"#,
            r#"{"DeadLetterQueue": 1}"#,
            r#"{"MaxReceiveCount": 0}"#,
        ];
        for attributes in cases {
            let line = format!(
                r#"{{"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {attributes}}}"#
            );
            assert_eq!(
                code(&line),
                ErrorCode::InvalidParameterValue,
                "attributes {attributes}"
            );
        }
    }

    #[test]
    fn send_message_parameter_values() {
        let long_body = "x".repeat(1025);
        let long_token = "a".repeat(129);
        let cases = [
            r#""Body": """#.to_owned(),
            format!(r#""Body": "{long_body}""#),
            r#""Body": "b", "DelaySeconds": 901"#.to_owned(),
            r#""Body": "b", "GroupId": "a.b""#.to_owned(),
            format!(r#""Body": "b", "DeduplicationId": "{long_token}""#),
            r#""Body": "b", "DeduplicationId": """#.to_owned(),
        ];
        for members in cases {
            let line = format!(r#"{{"Command": "SendMessage", "QueueName": "q", {members}}}"#);
            assert_eq!(
                code(&line),
                ErrorCode::InvalidParameterValue,
                "members {members}"
            );
        }
    }

    #[test]
    fn body_length_counts_utf8_bytes() {
        // 512 two-byte characters fill the 1024-byte limit exactly.
        let body = "\u{e9}".repeat(512);
        let line = format!(r#"{{"Command": "SendMessage", "QueueName": "q", "Body": "{body}"}}"#);
        assert!(matches!(
            parse(line.as_bytes()),
            Ok(Request::SendMessage(_))
        ));
        let line = format!(r#"{{"Command": "SendMessage", "QueueName": "q", "Body": "{body}a"}}"#);
        assert_eq!(code(&line), ErrorCode::InvalidParameterValue);
    }

    #[test]
    fn receive_message_defaults_to_one_message() {
        let line = r#"{"Command": "ReceiveMessage", "QueueName": "q"}"#;
        assert_eq!(
            parse(line.as_bytes()),
            Ok(Request::ReceiveMessage {
                queue: QueueName::new("q"),
                max_messages: 1,
                visibility_timeout: None,
            })
        );
        let line = r#"{"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 11}"#;
        assert_eq!(code(line), ErrorCode::InvalidParameterValue);
    }

    #[test]
    fn receipt_handle_must_be_a_string() {
        let line = r#"{"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": 1}"#;
        assert_eq!(code(line), ErrorCode::InvalidParameterValue);
        let line =
            r#"{"Command": "ChangeMessageVisibility", "QueueName": "q", "ReceiptHandle": "r-1"}"#;
        assert_eq!(code(line), ErrorCode::MissingParameter);
    }
}
