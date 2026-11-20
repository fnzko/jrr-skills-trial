//! Parsing of one input line into a validated [`Command`].
//!
//! This implements steps 1-3 of the error order in the spec (request shape,
//! required parameters, parameter values). Everything that needs queue state
//! happens later, in the broker.

use serde_json::{Map, Value};

use crate::error::{Error, ErrorCode, Result};

pub const MAX_QUEUE_NAME_LEN: usize = 80;
const FIFO_SUFFIX: &str = ".fifo";
const MAX_ID_LEN: usize = 128;
const MAX_BODY_BYTES: usize = 1024;

/// Attributes given in `CreateQueue` or `SetQueueAttributes`; absent ones are `None`.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Attributes {
    pub visibility_timeout: Option<u32>,
    pub delay_seconds: Option<u32>,
    pub retention_seconds: Option<u32>,
    pub fifo: Option<bool>,
    pub content_based_deduplication: Option<bool>,
    pub dead_letter_queue: Option<String>,
    pub max_receive_count: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    CreateQueue {
        name: String,
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
        seconds: u32,
    },
    SendMessage(SendMessage),
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendMessage {
    pub queue: String,
    pub body: String,
    pub delay_seconds: Option<u32>,
    pub group_id: Option<String>,
    pub deduplication_id: Option<String>,
}

/// Parses one raw input line (without its line terminator).
pub fn parse(line: &[u8]) -> Result<Command> {
    let value: Value = serde_json::from_slice(line)
        .map_err(|e| invalid_request(format!("line is not valid JSON: {e}")))?;
    let Value::Object(object) = value else {
        return Err(invalid_request("line is not a JSON object"));
    };
    let name = match object.get("Command") {
        Some(Value::String(name)) => name.as_str(),
        _ => return Err(invalid_request("`Command` must be a string")),
    };
    let params = Params(&object);

    match name {
        "CreateQueue" => {
            let name = params.require("QueueName")?;
            let attributes = params.optional("Attributes");
            // The name is checked before every other parameter.
            let name = queue_name_for_create(name)?;
            let attributes = attributes
                .map(parse_attributes)
                .transpose()?
                .unwrap_or_default();
            Ok(Command::CreateQueue { name, attributes })
        }
        "SetQueueAttributes" => {
            let queue = params.require("QueueName")?;
            let attributes = params.require("Attributes")?;
            Ok(Command::SetQueueAttributes {
                queue: string("QueueName", queue)?,
                attributes: parse_attributes(attributes)?,
            })
        }
        "GetQueueAttributes" => Ok(Command::GetQueueAttributes {
            queue: params.queue()?,
        }),
        "ListQueues" => Ok(Command::ListQueues {
            prefix: params
                .optional("Prefix")
                .map(|v| string("Prefix", v))
                .transpose()?,
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
                seconds: integer("Seconds", seconds, 0, 1_209_600)?,
            })
        }
        "SendMessage" => {
            let queue = params.require("QueueName")?;
            let body = params.require("Body")?;
            Ok(Command::SendMessage(SendMessage {
                queue: string("QueueName", queue)?,
                body: message_body(body)?,
                delay_seconds: params
                    .optional("DelaySeconds")
                    .map(|v| integer("DelaySeconds", v, 0, 900))
                    .transpose()?,
                group_id: params
                    .optional("GroupId")
                    .map(|v| message_id("GroupId", v))
                    .transpose()?,
                deduplication_id: params
                    .optional("DeduplicationId")
                    .map(|v| message_id("DeduplicationId", v))
                    .transpose()?,
            }))
        }
        "ReceiveMessage" => {
            let queue = params.require("QueueName")?;
            Ok(Command::ReceiveMessage {
                queue: string("QueueName", queue)?,
                max_messages: params
                    .optional("MaxMessages")
                    .map(|v| integer("MaxMessages", v, 1, 10))
                    .transpose()?
                    .map_or(1, |n: u32| n as usize),
                visibility_timeout: params
                    .optional("VisibilityTimeout")
                    .map(|v| integer("VisibilityTimeout", v, 0, 43_200))
                    .transpose()?,
            })
        }
        "DeleteMessage" => {
            let queue = params.require("QueueName")?;
            let handle = params.require("ReceiptHandle")?;
            Ok(Command::DeleteMessage {
                queue: string("QueueName", queue)?,
                receipt_handle: string("ReceiptHandle", handle)?,
            })
        }
        "ChangeMessageVisibility" => {
            let queue = params.require("QueueName")?;
            let handle = params.require("ReceiptHandle")?;
            let timeout = params.require("VisibilityTimeout")?;
            Ok(Command::ChangeMessageVisibility {
                queue: string("QueueName", queue)?,
                receipt_handle: string("ReceiptHandle", handle)?,
                visibility_timeout: integer("VisibilityTimeout", timeout, 0, 43_200)?,
            })
        }
        other => Err(invalid_request(format!("unknown command `{other}`"))),
    }
}

/// Returns whether `name` follows the queue naming rules of the spec.
pub fn is_valid_queue_name(name: &str) -> bool {
    let is_name_char = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '-';
    let stem = name.strip_suffix(FIFO_SUFFIX).unwrap_or(name);
    !stem.is_empty() && name.chars().count() <= MAX_QUEUE_NAME_LEN && stem.chars().all(is_name_char)
}

/// Whether a queue name denotes a FIFO queue.
pub fn is_fifo_name(name: &str) -> bool {
    name.ends_with(FIFO_SUFFIX)
}

fn invalid_request(message: impl Into<String>) -> Error {
    Error::new(ErrorCode::InvalidRequest, message)
}

fn invalid_value(message: impl Into<String>) -> Error {
    Error::new(ErrorCode::InvalidParameterValue, message)
}

/// Member lookup that treats `null` like an absent member.
struct Params<'a>(&'a Map<String, Value>);

impl<'a> Params<'a> {
    fn optional(&self, name: &str) -> Option<&'a Value> {
        self.0.get(name).filter(|v| !v.is_null())
    }

    fn require(&self, name: &str) -> Result<&'a Value> {
        self.optional(name).ok_or_else(|| {
            Error::new(
                ErrorCode::MissingParameter,
                format!("parameter `{name}` is required"),
            )
        })
    }

    fn queue(&self) -> Result<String> {
        string("QueueName", self.require("QueueName")?)
    }
}

fn string(name: &str, value: &Value) -> Result<String> {
    value
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| invalid_value(format!("`{name}` must be a string")))
}

fn boolean(name: &str, value: &Value) -> Result<bool> {
    value
        .as_bool()
        .ok_or_else(|| invalid_value(format!("`{name}` must be a boolean")))
}

/// An integer literal (no fraction, no exponent) within `min..=max`.
fn integer(name: &str, value: &Value, min: u32, max: u32) -> Result<u32> {
    let Some(n) = value.as_i64() else {
        return Err(invalid_value(format!("`{name}` must be an integer")));
    };
    u32::try_from(n)
        .ok()
        .filter(|n| (min..=max).contains(n))
        .ok_or_else(|| invalid_value(format!("`{name}` must be between {min} and {max}")))
}

fn queue_name_for_create(value: &Value) -> Result<String> {
    let name = string("QueueName", value)?;
    if is_valid_queue_name(&name) {
        Ok(name)
    } else {
        Err(Error::new(
            ErrorCode::QueueNameInvalid,
            "queue names are 1-80 characters of A-Z a-z 0-9 _ -, optionally followed by `.fifo`",
        ))
    }
}

fn message_body(value: &Value) -> Result<String> {
    let body = string("Body", value)?;
    if (1..=MAX_BODY_BYTES).contains(&body.len()) {
        Ok(body)
    } else {
        Err(invalid_value(format!(
            "`Body` must be 1 to {MAX_BODY_BYTES} bytes long"
        )))
    }
}

/// `GroupId` / `DeduplicationId`: 1 to 128 characters from `A-Z a-z 0-9 _ -`.
fn message_id(name: &str, value: &Value) -> Result<String> {
    let id = string(name, value)?;
    let valid = (1..=MAX_ID_LEN).contains(&id.len())
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if valid {
        Ok(id)
    } else {
        Err(invalid_value(format!(
            "`{name}` must be 1 to {MAX_ID_LEN} characters from A-Z a-z 0-9 _ -"
        )))
    }
}

fn parse_attributes(value: &Value) -> Result<Attributes> {
    let Value::Object(object) = value else {
        return Err(invalid_value("`Attributes` must be an object"));
    };
    let mut attributes = Attributes::default();
    for (name, value) in object {
        if value.is_null() {
            // Null counts as absent, but the name must still be one we know.
            check_attribute_name(name)?;
            continue;
        }
        match name.as_str() {
            "VisibilityTimeout" => {
                attributes.visibility_timeout = Some(integer(name, value, 0, 43_200)?)
            }
            "DelaySeconds" => attributes.delay_seconds = Some(integer(name, value, 0, 900)?),
            "RetentionSeconds" => {
                attributes.retention_seconds = Some(integer(name, value, 60, 1_209_600)?)
            }
            "Fifo" => attributes.fifo = Some(boolean(name, value)?),
            "ContentBasedDeduplication" => {
                attributes.content_based_deduplication = Some(boolean(name, value)?)
            }
            "DeadLetterQueue" => attributes.dead_letter_queue = Some(string(name, value)?),
            "MaxReceiveCount" => {
                attributes.max_receive_count = Some(integer(name, value, 1, 1000)?)
            }
            _ => return Err(unknown_attribute(name)),
        }
    }
    Ok(attributes)
}

fn check_attribute_name(name: &str) -> Result<()> {
    const KNOWN: [&str; 7] = [
        "VisibilityTimeout",
        "DelaySeconds",
        "RetentionSeconds",
        "Fifo",
        "ContentBasedDeduplication",
        "DeadLetterQueue",
        "MaxReceiveCount",
    ];
    if KNOWN.contains(&name) {
        Ok(())
    } else {
        Err(unknown_attribute(name))
    }
}

fn unknown_attribute(name: &str) -> Error {
    invalid_value(format!("unknown attribute `{name}`"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code(line: &str) -> ErrorCode {
        parse(line.as_bytes()).expect_err("should fail").code
    }

    #[test]
    fn request_shape_errors() {
        for line in [
            "",
            "   ",
            "[]",
            "\"x\"",
            "{",
            "{} trailing",
            "{}",
            r#"{"Command": 1}"#,
            r#"{"Command": null}"#,
            r#"{"command": "Tick", "Seconds": 1}"#,
            r#"{"Command": "Nope"}"#,
        ] {
            assert_eq!(code(line), ErrorCode::InvalidRequest, "{line}");
        }
        assert_eq!(
            parse(b"\xff\xfe").unwrap_err().code,
            ErrorCode::InvalidRequest
        );
    }

    #[test]
    fn required_parameters_come_before_value_checks() {
        // The wrongly typed Body must lose against the missing QueueName.
        assert_eq!(
            code(r#"{"Command": "SendMessage", "Body": 5}"#),
            ErrorCode::MissingParameter
        );
        assert_eq!(
            code(r#"{"Command": "Tick", "Seconds": null}"#),
            ErrorCode::MissingParameter
        );
    }

    #[test]
    fn integers_must_be_plain_literals() {
        for literal in ["30.0", "3e1", "\"30\"", "true", "-1", "1209601", "1e400"] {
            let line = format!(r#"{{"Command": "Tick", "Seconds": {literal}}}"#);
            let err = parse(line.as_bytes()).unwrap_err().code;
            assert!(
                matches!(
                    err,
                    ErrorCode::InvalidParameterValue | ErrorCode::InvalidRequest
                ),
                "{literal}: {err:?}"
            );
        }
        assert_eq!(
            parse(br#"{"Command": "Tick", "Seconds": 30}"#),
            Ok(Command::Tick { seconds: 30 })
        );
        assert_eq!(
            parse(br#"{"Command": "Tick", "Seconds": -0}"#),
            Ok(Command::Tick { seconds: 0 })
        );
    }

    #[test]
    fn create_queue_checks_the_name_first() {
        assert_eq!(
            code(r#"{"Command": "CreateQueue", "QueueName": "a b", "Attributes": 3}"#),
            ErrorCode::QueueNameInvalid
        );
        assert_eq!(
            code(r#"{"Command": "CreateQueue", "QueueName": "a", "Attributes": 3}"#),
            ErrorCode::InvalidParameterValue
        );
        assert_eq!(
            code(r#"{"Command": "CreateQueue", "QueueName": 3}"#),
            ErrorCode::InvalidParameterValue
        );
    }

    #[test]
    fn queue_name_syntax() {
        let long = "a".repeat(80);
        let too_long = "a".repeat(81);
        assert!(is_valid_queue_name("jobs"));
        assert!(is_valid_queue_name("A-z_9"));
        assert!(is_valid_queue_name("a.fifo"));
        assert!(is_valid_queue_name(&long));
        assert!(is_valid_queue_name(&format!("{}.fifo", "a".repeat(75))));
        assert!(!is_valid_queue_name(&format!("{}.fifo", "a".repeat(76))));
        assert!(!is_valid_queue_name(&too_long));
        for bad in ["", ".fifo", "a.b", "a.fifo.fifo", "é", "a b", "a.fifo "] {
            assert!(!is_valid_queue_name(bad), "{bad:?}");
        }
    }

    #[test]
    fn null_attributes_are_absent_but_names_are_still_checked() {
        let ok = parse(br#"{"Command": "CreateQueue", "QueueName": "q", "Attributes": {"DelaySeconds": null}}"#);
        assert!(matches!(ok, Ok(Command::CreateQueue { .. })));
        assert_eq!(
            code(r#"{"Command": "CreateQueue", "QueueName": "q", "Attributes": {"Bogus": null}}"#),
            ErrorCode::InvalidParameterValue
        );
    }

    #[test]
    fn attribute_ranges() {
        for attr in [
            r#""VisibilityTimeout": 43201"#,
            r#""DelaySeconds": 901"#,
            r#""RetentionSeconds": 59"#,
            r#""MaxReceiveCount": 0"#,
            r#""Fifo": "true""#,
            r#""DeadLetterQueue": 1"#,
        ] {
            let line = format!(
                r#"{{"Command": "CreateQueue", "QueueName": "q", "Attributes": {{{attr}}}}}"#
            );
            assert_eq!(code(&line), ErrorCode::InvalidParameterValue, "{attr}");
        }
    }

    #[test]
    fn message_field_limits() {
        let send = |extra: &str| {
            parse(format!(r#"{{"Command": "SendMessage", "QueueName": "q", {extra}}}"#).as_bytes())
        };
        assert!(send(r#""Body": "x""#).is_ok());
        assert_eq!(
            send(r#""Body": """#).unwrap_err().code,
            ErrorCode::InvalidParameterValue
        );
        let big = "é".repeat(513); // 1026 bytes
        assert_eq!(
            send(&format!(r#""Body": "{big}""#)).unwrap_err().code,
            ErrorCode::InvalidParameterValue
        );
        assert_eq!(
            send(r#""Body": "x", "GroupId": "a b""#).unwrap_err().code,
            ErrorCode::InvalidParameterValue
        );
        assert_eq!(
            send(r#""Body": "x", "DeduplicationId": """#)
                .unwrap_err()
                .code,
            ErrorCode::InvalidParameterValue
        );
    }
}
