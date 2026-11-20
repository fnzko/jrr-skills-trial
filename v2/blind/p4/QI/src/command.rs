//! Reads one command line into a typed `Command`. The checks here are the first three of SPEC
//! section 2, in the order given there: request shape, required parameters, then each value.
//! Rules that need queue state run later, in the broker.

use crate::attributes::AttributeUpdate;
use crate::error::{ErrorCode, Failure};
use crate::ids::{DeduplicationId, GroupId, QueueName};
use crate::params::{bounded, invalid, missing, string};
use crate::time::Seconds;
use serde_json::{Map, Value};

/// One parsed command, with its parameters checked and defaults applied where the request
/// left them out but the default is needed to act. Parameters with no default stay optional.
#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    CreateQueue {
        name: QueueName,
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
        max_messages: u32,
        visibility_timeout: Option<Seconds>,
    },
    DeleteMessage {
        queue: String,
        receipt: String,
    },
    ChangeMessageVisibility {
        queue: String,
        receipt: String,
        visibility_timeout: Seconds,
    },
}

/// Reads one command line, without its newline.
pub fn parse(line: &[u8]) -> Result<Command, Failure> {
    let text = std::str::from_utf8(line).map_err(|_| request_error("line is not valid UTF-8"))?;
    let value: Value = serde_json::from_str(text)
        .map_err(|error| request_error(format!("line is not valid JSON: {error}")))?;
    let Value::Object(members) = value else {
        return Err(request_error("line is not a JSON object"));
    };
    let Some(Value::String(name)) = members.get("Command") else {
        return Err(request_error("member Command is missing or not a string"));
    };
    let params = Params(&members);
    match name.as_str() {
        "CreateQueue" => create_queue(&params),
        "SetQueueAttributes" => set_queue_attributes(&params),
        "GetQueueAttributes" => queue_only(&params, |queue| Command::GetQueueAttributes { queue }),
        "ListQueues" => list_queues(&params),
        "DeleteQueue" => queue_only(&params, |queue| Command::DeleteQueue { queue }),
        "PurgeQueue" => queue_only(&params, |queue| Command::PurgeQueue { queue }),
        "Tick" => tick(&params),
        "SendMessage" => send_message(&params),
        "ReceiveMessage" => receive_message(&params),
        "DeleteMessage" => delete_message(&params),
        "ChangeMessageVisibility" => change_message_visibility(&params),
        other => Err(request_error(format!("unknown command {other}"))),
    }
}

fn request_error(message: impl Into<String>) -> Failure {
    Failure::new(ErrorCode::InvalidRequest, message)
}

/// The members of a command line. A member whose value is null counts as absent.
struct Params<'a>(&'a Map<String, Value>);

impl<'a> Params<'a> {
    fn get(&self, name: &str) -> Option<&'a Value> {
        self.0.get(name).filter(|value| !value.is_null())
    }

    fn required(&self, name: &str) -> Result<&'a Value, Failure> {
        self.get(name).ok_or_else(|| missing(name))
    }
}

/// A string parameter that must parse as a word of a particular kind.
fn word<T>(name: &str, value: &Value, parse: impl Fn(&str) -> Option<T>) -> Result<T, Failure> {
    let text = string(name, value)?;
    parse(text).ok_or_else(|| {
        invalid(
            name,
            format!("must be 1 to 128 characters from A-Z a-z 0-9 _ -, got {text:?}"),
        )
    })
}

/// The `QueueName` parameter of a command that only looks the queue up. A name that breaks the
/// naming rules cannot match a queue, so it is passed on as an ordinary string.
fn queue_name_arg(params: &Params<'_>) -> Result<String, Failure> {
    Ok(string("QueueName", params.required("QueueName")?)?.to_owned())
}

fn queue_only(
    params: &Params<'_>,
    make: impl FnOnce(String) -> Command,
) -> Result<Command, Failure> {
    Ok(make(queue_name_arg(params)?))
}

fn create_queue(params: &Params<'_>) -> Result<Command, Failure> {
    // QueueName is checked before every other parameter, and a bad name is its own code.
    let text = string("QueueName", params.required("QueueName")?)?;
    let name = QueueName::parse(text).ok_or_else(|| {
        Failure::new(
            ErrorCode::QueueNameInvalid,
            format!("queue name {text:?} breaks the naming rules"),
        )
    })?;
    let attributes = AttributeUpdate::parse(params.get("Attributes"))?;
    Ok(Command::CreateQueue { name, attributes })
}

fn set_queue_attributes(params: &Params<'_>) -> Result<Command, Failure> {
    let queue_value = params.required("QueueName")?;
    let attributes_value = params.required("Attributes")?;
    let queue = string("QueueName", queue_value)?.to_owned();
    let attributes = AttributeUpdate::parse(Some(attributes_value))?;
    Ok(Command::SetQueueAttributes { queue, attributes })
}

fn list_queues(params: &Params<'_>) -> Result<Command, Failure> {
    let prefix = params
        .get("Prefix")
        .map(|value| string("Prefix", value).map(str::to_owned))
        .transpose()?;
    Ok(Command::ListQueues { prefix })
}

fn tick(params: &Params<'_>) -> Result<Command, Failure> {
    let value = params.required("Seconds")?;
    let seconds = Seconds::new(bounded("Seconds", value, 0, 1_209_600)?);
    Ok(Command::Tick { seconds })
}

fn send_message(params: &Params<'_>) -> Result<Command, Failure> {
    let queue_value = params.required("QueueName")?;
    let body_value = params.required("Body")?;
    let queue = string("QueueName", queue_value)?.to_owned();
    let body = string("Body", body_value)?;
    if !(1..=1024).contains(&body.len()) {
        return Err(invalid("Body", "must be 1 to 1024 bytes"));
    }
    let delay = params
        .get("DelaySeconds")
        .map(|value| bounded("DelaySeconds", value, 0, 900).map(Seconds::new))
        .transpose()?;
    let group = params
        .get("GroupId")
        .map(|value| word("GroupId", value, GroupId::parse))
        .transpose()?;
    let deduplication = params
        .get("DeduplicationId")
        .map(|value| word("DeduplicationId", value, DeduplicationId::parse))
        .transpose()?;
    Ok(Command::SendMessage {
        queue,
        body: body.to_owned(),
        delay,
        group,
        deduplication,
    })
}

fn receive_message(params: &Params<'_>) -> Result<Command, Failure> {
    let queue = queue_name_arg(params)?;
    let max_messages = params
        .get("MaxMessages")
        .map(|value| bounded("MaxMessages", value, 1, 10))
        .transpose()?
        .unwrap_or(1);
    let visibility_timeout = params
        .get("VisibilityTimeout")
        .map(|value| bounded("VisibilityTimeout", value, 0, 43_200).map(Seconds::new))
        .transpose()?;
    Ok(Command::ReceiveMessage {
        queue,
        max_messages,
        visibility_timeout,
    })
}

fn delete_message(params: &Params<'_>) -> Result<Command, Failure> {
    let queue_value = params.required("QueueName")?;
    let receipt_value = params.required("ReceiptHandle")?;
    let queue = string("QueueName", queue_value)?.to_owned();
    let receipt = string("ReceiptHandle", receipt_value)?.to_owned();
    Ok(Command::DeleteMessage { queue, receipt })
}

fn change_message_visibility(params: &Params<'_>) -> Result<Command, Failure> {
    let queue_value = params.required("QueueName")?;
    let receipt_value = params.required("ReceiptHandle")?;
    let timeout_value = params.required("VisibilityTimeout")?;
    let queue = string("QueueName", queue_value)?.to_owned();
    let receipt = string("ReceiptHandle", receipt_value)?.to_owned();
    let visibility_timeout = Seconds::new(bounded("VisibilityTimeout", timeout_value, 0, 43_200)?);
    Ok(Command::ChangeMessageVisibility {
        queue,
        receipt,
        visibility_timeout,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outcome(line: &str) -> Result<Command, ErrorCode> {
        parse(line.as_bytes()).map_err(|failure| failure.code())
    }

    #[test]
    fn well_formed_lines_parse_to_their_command() {
        let cases: Vec<(&str, Result<Command, ErrorCode>)> = vec![
            (
                r#"{"Command": "Tick", "Seconds": 30}"#,
                Ok(Command::Tick {
                    seconds: Seconds::new(30),
                }),
            ),
            (
                r#"{"Command": "Tick", "Seconds": 1, "Unused": [1, 2]}"#,
                Ok(Command::Tick {
                    seconds: Seconds::new(1),
                }),
            ),
            (
                r#"{"Command": "ListQueues", "Prefix": null}"#,
                Ok(Command::ListQueues { prefix: None }),
            ),
            (
                r#"{"Command": "ReceiveMessage", "QueueName": "q"}"#,
                Ok(Command::ReceiveMessage {
                    queue: "q".into(),
                    max_messages: 1,
                    visibility_timeout: None,
                }),
            ),
            (
                r#"{"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 10, "VisibilityTimeout": 0}"#,
                Ok(Command::ReceiveMessage {
                    queue: "q".into(),
                    max_messages: 10,
                    visibility_timeout: Some(Seconds::new(0)),
                }),
            ),
            (
                r#"{"Command": "SendMessage", "QueueName": "q", "Body": "hi", "GroupId": "g"}"#,
                Ok(Command::SendMessage {
                    queue: "q".into(),
                    body: "hi".into(),
                    delay: None,
                    group: GroupId::parse("g"),
                    deduplication: None,
                }),
            ),
        ];
        for (line, expected) in cases {
            assert_eq!(outcome(line), expected, "{line}");
        }
    }

    #[test]
    fn line_shape_failures_are_invalid_requests() {
        let cases = [
            "",
            "   ",
            "[1]",
            "42",
            "{",
            r#"{}"#,
            r#"{"Command": 5}"#,
            r#"{"Command": null}"#,
            r#"{"command": "Tick", "Seconds": 1}"#,
            r#"{"Command": "Nope"}"#,
            r#"{"Command": "tick", "Seconds": 1}"#,
        ];
        for line in cases {
            assert_eq!(outcome(line), Err(ErrorCode::InvalidRequest), "{line:?}");
        }
    }

    #[test]
    fn non_utf8_lines_are_invalid_requests() {
        let result = parse(b"{\"Command\": \"Tick\", \"Seconds\": \"\xff\"}")
            .map_err(|failure| failure.code());
        assert_eq!(result, Err(ErrorCode::InvalidRequest));
    }

    #[test]
    fn missing_and_null_required_parameters_come_before_value_checks() {
        let cases = [
            (r#"{"Command": "Tick"}"#, ErrorCode::MissingParameter),
            (
                r#"{"Command": "Tick", "Seconds": null}"#,
                ErrorCode::MissingParameter,
            ),
            (
                r#"{"Command": "SendMessage", "QueueName": 5}"#,
                ErrorCode::MissingParameter,
            ),
            (
                r#"{"Command": "DeleteMessage", "QueueName": "q"}"#,
                ErrorCode::MissingParameter,
            ),
            (
                r#"{"Command": "ChangeMessageVisibility", "QueueName": "q", "ReceiptHandle": 9}"#,
                ErrorCode::MissingParameter,
            ),
            (
                r#"{"Command": "CreateQueue", "QueueName": "bad name"}"#,
                ErrorCode::QueueNameInvalid,
            ),
            (
                r#"{"Command": "CreateQueue", "QueueName": 5, "Attributes": 1}"#,
                ErrorCode::InvalidParameterValue,
            ),
            (
                r#"{"Command": "CreateQueue", "QueueName": "bad name", "Attributes": 1}"#,
                ErrorCode::QueueNameInvalid,
            ),
            (
                r#"{"Command": "CreateQueue", "Attributes": {}}"#,
                ErrorCode::MissingParameter,
            ),
            (
                r#"{"Command": "SetQueueAttributes", "QueueName": "q"}"#,
                ErrorCode::MissingParameter,
            ),
            (
                r#"{"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": 1}"#,
                ErrorCode::InvalidParameterValue,
            ),
        ];
        for (line, code) in cases {
            assert_eq!(outcome(line), Err(code), "{line}");
        }
    }

    #[test]
    fn parameter_values_are_checked_for_type_and_range() {
        let cases = [
            (
                r#"{"Command": "Tick", "Seconds": 30.0}"#,
                ErrorCode::InvalidParameterValue,
            ),
            (
                r#"{"Command": "Tick", "Seconds": "30"}"#,
                ErrorCode::InvalidParameterValue,
            ),
            (
                r#"{"Command": "Tick", "Seconds": 1209601}"#,
                ErrorCode::InvalidParameterValue,
            ),
            (
                r#"{"Command": "ListQueues", "Prefix": 1}"#,
                ErrorCode::InvalidParameterValue,
            ),
            (
                r#"{"Command": "SendMessage", "QueueName": "q", "Body": ""}"#,
                ErrorCode::InvalidParameterValue,
            ),
            (
                r#"{"Command": "SendMessage", "QueueName": "q", "Body": "x", "DelaySeconds": 901}"#,
                ErrorCode::InvalidParameterValue,
            ),
            (
                r#"{"Command": "SendMessage", "QueueName": "q", "Body": "x", "GroupId": "a b"}"#,
                ErrorCode::InvalidParameterValue,
            ),
            (
                r#"{"Command": "SendMessage", "QueueName": "q", "Body": "x", "DeduplicationId": ""}"#,
                ErrorCode::InvalidParameterValue,
            ),
            (
                r#"{"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 0}"#,
                ErrorCode::InvalidParameterValue,
            ),
            (
                r#"{"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 11}"#,
                ErrorCode::InvalidParameterValue,
            ),
            (
                r#"{"Command": "ChangeMessageVisibility", "QueueName": "q", "ReceiptHandle": "r-1", "VisibilityTimeout": 43201}"#,
                ErrorCode::InvalidParameterValue,
            ),
        ];
        for (line, code) in cases {
            assert_eq!(outcome(line), Err(code), "{line}");
        }
    }

    #[test]
    fn body_length_limit_is_1024_bytes() {
        let body = |n: usize| {
            format!(
                r#"{{"Command": "SendMessage", "QueueName": "q", "Body": "{}"}}"#,
                "a".repeat(n)
            )
        };
        assert_eq!(outcome(&body(1024)).map(|_| ()), Ok(()));
        assert_eq!(outcome(&body(1025)), Err(ErrorCode::InvalidParameterValue));
    }
}
