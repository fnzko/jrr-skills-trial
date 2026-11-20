//! Parsing of one command line into a typed `Command`.
//!
//! This applies checks 1 to 3 of section 2 of SPEC.md: request shape, then
//! required parameters, then parameter values. Checks that need queue state
//! run later, in the broker, in the order the specification gives.

use serde_json::{Map, Value};

use crate::attributes::AttributeUpdates;
use crate::error::{Error, ErrorCode, Result};
use crate::types::{DeduplicationId, GroupId, QueueName};
use crate::wire;

const MAX_BODY_BYTES: usize = 1024;
const MAX_DELAY_SECONDS: u64 = 900;
const MAX_TICK_SECONDS: u64 = 1_209_600;
const MAX_VISIBILITY_TIMEOUT: u64 = 43_200;
const MAX_RECEIVE_MESSAGES: u64 = 10;

/// A parsed command. Queue names are plain strings here, because a name that
/// is not a valid queue name still reaches the lookup and gets
/// `QueueDoesNotExist`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    CreateQueue {
        name: QueueName,
        attributes: AttributeUpdates,
    },
    SetQueueAttributes {
        queue: String,
        attributes: AttributeUpdates,
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
        receipt: String,
    },
    ChangeMessageVisibility {
        queue: String,
        receipt: String,
        visibility_timeout: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendMessage {
    pub queue: String,
    pub body: String,
    pub delay_seconds: Option<u64>,
    pub group_id: Option<GroupId>,
    pub deduplication_id: Option<DeduplicationId>,
}

impl Command {
    /// Parses one line of input. The line must be UTF-8 and, as a whole, one
    /// JSON object with a string `Command` member.
    pub fn parse_line(line: &[u8]) -> Result<Command> {
        let text =
            std::str::from_utf8(line).map_err(|_| request_error("the line is not valid UTF-8"))?;
        let value: Value = serde_json::from_str(text)
            .map_err(|error| request_error(format!("the line is not JSON: {error}")))?;
        let Value::Object(object) = value else {
            return Err(request_error("the line is not a JSON object"));
        };
        let params = Params(&object);
        let name = params
            .present("Command")
            .and_then(Value::as_str)
            .ok_or_else(|| request_error("the object has no string Command member"))?;
        match name {
            "CreateQueue" => create_queue(&params),
            "SetQueueAttributes" => set_queue_attributes(&params),
            "GetQueueAttributes" => {
                let queue = queue_name(&params)?;
                Ok(Command::GetQueueAttributes { queue })
            }
            "ListQueues" => list_queues(&params),
            "DeleteQueue" => Ok(Command::DeleteQueue {
                queue: queue_name(&params)?,
            }),
            "PurgeQueue" => Ok(Command::PurgeQueue {
                queue: queue_name(&params)?,
            }),
            "Tick" => tick(&params),
            "SendMessage" => send_message(&params).map(Command::SendMessage),
            "ReceiveMessage" => receive_message(&params),
            "DeleteMessage" => delete_message(&params),
            "ChangeMessageVisibility" => change_message_visibility(&params),
            other => Err(request_error(format!("unknown command {other}"))),
        }
    }
}

fn request_error(message: impl Into<String>) -> Error {
    Error::new(ErrorCode::InvalidRequest, message)
}

/// The members of a command object. A member whose value is `null` is absent.
struct Params<'a>(&'a Map<String, Value>);

impl<'a> Params<'a> {
    fn present(&self, name: &str) -> Option<&'a Value> {
        self.0.get(name).filter(|value| !value.is_null())
    }

    fn required(&self, name: &str) -> Result<&'a Value> {
        self.present(name)
            .ok_or_else(|| Error::new(ErrorCode::MissingParameter, format!("{name} is required")))
    }
}

fn queue_name(params: &Params) -> Result<String> {
    let value = params.required("QueueName")?;
    Ok(wire::string(value, "QueueName")?.to_owned())
}

fn create_queue(params: &Params) -> Result<Command> {
    let value = params.required("QueueName")?;
    // The name syntax is checked before any other parameter value.
    let text = wire::string(value, "QueueName")?;
    let name = QueueName::parse(text).ok_or_else(|| {
        Error::new(
            ErrorCode::QueueNameInvalid,
            format!("queue name {text:?} breaks the naming rules"),
        )
    })?;
    let attributes = match params.present("Attributes") {
        Some(value) => AttributeUpdates::from_json(value)?,
        None => AttributeUpdates::default(),
    };
    Ok(Command::CreateQueue { name, attributes })
}

fn set_queue_attributes(params: &Params) -> Result<Command> {
    let queue_value = params.required("QueueName")?;
    let attributes_value = params.required("Attributes")?;
    let queue = wire::string(queue_value, "QueueName")?.to_owned();
    let attributes = AttributeUpdates::from_json(attributes_value)?;
    Ok(Command::SetQueueAttributes { queue, attributes })
}

fn list_queues(params: &Params) -> Result<Command> {
    let prefix = match params.present("Prefix") {
        Some(value) => Some(wire::string(value, "Prefix")?.to_owned()),
        None => None,
    };
    Ok(Command::ListQueues { prefix })
}

fn tick(params: &Params) -> Result<Command> {
    let value = params.required("Seconds")?;
    let seconds = wire::integer_in(value, "Seconds", 0, MAX_TICK_SECONDS)?;
    Ok(Command::Tick { seconds })
}

fn send_message(params: &Params) -> Result<SendMessage> {
    let queue_value = params.required("QueueName")?;
    let body_value = params.required("Body")?;
    let queue = wire::string(queue_value, "QueueName")?.to_owned();
    let body = wire::string(body_value, "Body")?;
    if body.is_empty() || body.len() > MAX_BODY_BYTES {
        return Err(wire::invalid(format!(
            "Body must be between 1 and {MAX_BODY_BYTES} bytes"
        )));
    }
    let delay_seconds = match params.present("DelaySeconds") {
        Some(value) => Some(wire::integer_in(
            value,
            "DelaySeconds",
            0,
            MAX_DELAY_SECONDS,
        )?),
        None => None,
    };
    let group_id = optional_token(params, "GroupId", GroupId::parse)?;
    let deduplication_id = optional_token(params, "DeduplicationId", DeduplicationId::parse)?;
    Ok(SendMessage {
        queue,
        body: body.to_owned(),
        delay_seconds,
        group_id,
        deduplication_id,
    })
}

/// Reads an optional identifier and checks it with `parse`.
fn optional_token<T>(
    params: &Params,
    name: &str,
    parse: impl FnOnce(&str) -> Option<T>,
) -> Result<Option<T>> {
    let Some(value) = params.present(name) else {
        return Ok(None);
    };
    let text = wire::string(value, name)?;
    parse(text).map(Some).ok_or_else(|| {
        wire::invalid(format!(
            "{name} must be 1 to 128 characters from A-Z a-z 0-9 _ -"
        ))
    })
}

fn receive_message(params: &Params) -> Result<Command> {
    let queue = queue_name(params)?;
    let max_messages = match params.present("MaxMessages") {
        Some(value) => wire::integer_in(value, "MaxMessages", 1, MAX_RECEIVE_MESSAGES)?,
        None => 1,
    };
    let visibility_timeout = match params.present("VisibilityTimeout") {
        Some(value) => Some(wire::integer_in(
            value,
            "VisibilityTimeout",
            0,
            MAX_VISIBILITY_TIMEOUT,
        )?),
        None => None,
    };
    Ok(Command::ReceiveMessage {
        queue,
        max_messages,
        visibility_timeout,
    })
}

fn delete_message(params: &Params) -> Result<Command> {
    let queue_value = params.required("QueueName")?;
    let receipt_value = params.required("ReceiptHandle")?;
    let queue = wire::string(queue_value, "QueueName")?.to_owned();
    let receipt = wire::string(receipt_value, "ReceiptHandle")?.to_owned();
    Ok(Command::DeleteMessage { queue, receipt })
}

fn change_message_visibility(params: &Params) -> Result<Command> {
    let queue_value = params.required("QueueName")?;
    let receipt_value = params.required("ReceiptHandle")?;
    let timeout_value = params.required("VisibilityTimeout")?;
    let queue = wire::string(queue_value, "QueueName")?.to_owned();
    let receipt = wire::string(receipt_value, "ReceiptHandle")?.to_owned();
    let visibility_timeout = wire::integer_in(
        timeout_value,
        "VisibilityTimeout",
        0,
        MAX_VISIBILITY_TIMEOUT,
    )?;
    Ok(Command::ChangeMessageVisibility {
        queue,
        receipt,
        visibility_timeout,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(line: &str) -> Result<Command> {
        Command::parse_line(line.as_bytes())
    }

    fn code(line: &str) -> ErrorCode {
        parse(line).unwrap_err().code()
    }

    #[test]
    fn parses_queue_commands() {
        assert_eq!(
            parse(r#"{"Command": "PurgeQueue", "QueueName": "jobs"}"#),
            Ok(Command::PurgeQueue {
                queue: "jobs".into()
            })
        );
        assert_eq!(
            parse(r#"{"Command": "ListQueues"}"#),
            Ok(Command::ListQueues { prefix: None })
        );
        assert_eq!(
            parse(r#"{"Command": "ListQueues", "Prefix": null}"#),
            Ok(Command::ListQueues { prefix: None })
        );
        assert_eq!(
            parse(r#"{"Command": "Tick", "Seconds": 0}"#),
            Ok(Command::Tick { seconds: 0 })
        );
    }

    #[test]
    fn create_queue_takes_a_valid_name_and_optional_attributes() {
        let command = parse(
            r#"{"Command": "CreateQueue", "QueueName": "jobs.fifo", "Attributes": {"Fifo": true}}"#,
        )
        .unwrap();
        let Command::CreateQueue { name, attributes } = command else {
            panic!("expected CreateQueue, got {command:?}");
        };
        assert_eq!(name.as_str(), "jobs.fifo");
        assert_eq!(attributes.fifo, Some(true));
    }

    #[test]
    fn create_queue_reports_a_bad_name_before_bad_attributes() {
        let line = r#"{"Command": "CreateQueue", "QueueName": "a b", "Attributes": {"Colour": 1}}"#;
        assert_eq!(code(line), ErrorCode::QueueNameInvalid);
    }

    #[test]
    fn missing_required_parameters_are_reported_before_wrong_values() {
        // QueueName has the wrong type, but Body is missing: presence comes first.
        let line = r#"{"Command": "SendMessage", "QueueName": 5}"#;
        assert_eq!(code(line), ErrorCode::MissingParameter);
    }

    #[test]
    fn null_required_parameter_counts_as_absent() {
        assert_eq!(
            code(r#"{"Command": "PurgeQueue", "QueueName": null}"#),
            ErrorCode::MissingParameter
        );
    }

    #[test]
    fn request_shape_errors_are_invalid_request() {
        let cases = [
            "",
            "   ",
            "[]",
            "\"Tick\"",
            "{not json",
            r#"{"QueueName": "jobs"}"#,
            r#"{"Command": null}"#,
            r#"{"Command": 5}"#,
            r#"{"Command": "Explode"}"#,
            r#"{"Command": "tick", "Seconds": 1}"#,
            r#"{"Command": "Tick", "Seconds": 1} x"#,
        ];
        for line in cases {
            assert_eq!(code(line), ErrorCode::InvalidRequest, "line {line:?}");
        }
        assert_eq!(
            Command::parse_line(b"\xff"),
            Err(request_error("the line is not valid UTF-8"))
        );
    }

    #[test]
    fn integers_must_be_plain_and_in_range() {
        assert_eq!(
            code(r#"{"Command": "Tick", "Seconds": 30.0}"#),
            ErrorCode::InvalidParameterValue
        );
        assert_eq!(
            code(r#"{"Command": "Tick", "Seconds": "30"}"#),
            ErrorCode::InvalidParameterValue
        );
        assert_eq!(
            code(r#"{"Command": "Tick", "Seconds": 1209601}"#),
            ErrorCode::InvalidParameterValue
        );
    }

    #[test]
    fn send_message_parses_optional_parameters() {
        let command = parse(
            r#"{"Command": "SendMessage", "QueueName": "q.fifo", "Body": "hi", "GroupId": "g1", "DeduplicationId": "d-1", "DelaySeconds": 900}"#,
        )
        .unwrap();
        assert_eq!(
            command,
            Command::SendMessage(SendMessage {
                queue: "q.fifo".into(),
                body: "hi".into(),
                delay_seconds: Some(900),
                group_id: GroupId::parse("g1"),
                deduplication_id: DeduplicationId::parse("d-1"),
            })
        );
    }

    #[test]
    fn send_message_checks_body_length_in_bytes() {
        let ok = format!(
            r#"{{"Command": "SendMessage", "QueueName": "q", "Body": "{}"}}"#,
            "a".repeat(1024)
        );
        assert!(parse(&ok).is_ok());
        let too_long = format!(
            r#"{{"Command": "SendMessage", "QueueName": "q", "Body": "{}"}}"#,
            "a".repeat(1025)
        );
        assert_eq!(code(&too_long), ErrorCode::InvalidParameterValue);
        // Two bytes of UTF-8 per character: 513 characters are 1026 bytes.
        let multibyte = format!(
            r#"{{"Command": "SendMessage", "QueueName": "q", "Body": "{}"}}"#,
            "\u{e9}".repeat(513)
        );
        assert_eq!(code(&multibyte), ErrorCode::InvalidParameterValue);
        assert_eq!(
            code(r#"{"Command": "SendMessage", "QueueName": "q", "Body": ""}"#),
            ErrorCode::InvalidParameterValue
        );
    }

    #[test]
    fn send_message_rejects_bad_group_id() {
        let line = r#"{"Command": "SendMessage", "QueueName": "q", "Body": "x", "GroupId": "a.b"}"#;
        assert_eq!(code(line), ErrorCode::InvalidParameterValue);
    }

    #[test]
    fn receive_message_defaults_max_messages_to_one() {
        assert_eq!(
            parse(r#"{"Command": "ReceiveMessage", "QueueName": "q"}"#),
            Ok(Command::ReceiveMessage {
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
    fn change_message_visibility_needs_all_three_parameters() {
        let line =
            r#"{"Command": "ChangeMessageVisibility", "QueueName": "q", "ReceiptHandle": "r-1"}"#;
        assert_eq!(code(line), ErrorCode::MissingParameter);
        assert_eq!(
            parse(
                r#"{"Command": "ChangeMessageVisibility", "QueueName": "q", "ReceiptHandle": "r-1", "VisibilityTimeout": 0}"#
            ),
            Ok(Command::ChangeMessageVisibility {
                queue: "q".into(),
                receipt: "r-1".into(),
                visibility_timeout: 0,
            })
        );
    }

    #[test]
    fn set_queue_attributes_requires_an_attributes_object() {
        let line = r#"{"Command": "SetQueueAttributes", "QueueName": "q"}"#;
        assert_eq!(code(line), ErrorCode::MissingParameter);
        let line = r#"{"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": 3}"#;
        assert_eq!(code(line), ErrorCode::InvalidParameterValue);
    }
}
