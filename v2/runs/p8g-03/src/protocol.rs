//! Turns a request line into a typed [`Request`], applying the shape,
//! required-parameter and parameter-value checks in the order of the spec.

use serde_json::{Map, Value};

use crate::attributes::{AttributeRequest, DeadLetterRequest};
use crate::error::{Error, ErrorCode, Result};
use crate::types::{
    Body, DedupKey, DeduplicationId, DelaySeconds, GroupId, MaxMessages, MaxReceiveCount,
    QueueName, Ranged, ReceiptHandle, RetentionSeconds, TickSeconds, VisibilityTimeout,
};

#[derive(Debug, PartialEq, Eq)]
pub enum Request {
    CreateQueue {
        name: QueueName,
        attributes: AttributeRequest,
    },
    SetQueueAttributes {
        queue: String,
        attributes: AttributeRequest,
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
        seconds: TickSeconds,
    },
    SendMessage(Send),
    ReceiveMessage {
        queue: String,
        max_messages: MaxMessages,
        visibility_timeout: Option<VisibilityTimeout>,
    },
    DeleteMessage {
        queue: String,
        handle: HandleText,
    },
    ChangeMessageVisibility {
        queue: String,
        handle: HandleText,
        visibility_timeout: VisibilityTimeout,
    },
}

#[derive(Debug, PartialEq, Eq)]
pub struct Send {
    pub queue: String,
    pub body: Body,
    pub delay: Option<DelaySeconds>,
    pub group: Option<GroupId>,
    pub deduplication: Option<DedupKey>,
}

/// A receipt handle as the caller wrote it. Text that miniq could never have
/// produced is kept apart, because it is reported differently from a handle
/// that was issued once.
#[derive(Debug, PartialEq, Eq)]
pub enum HandleText {
    Canonical(ReceiptHandle),
    Malformed,
}

pub fn parse(line: &[u8]) -> Result<Request> {
    let text = std::str::from_utf8(line)
        .map_err(|e| Error::invalid_request(format!("line is not valid UTF-8: {e}")))?;
    let value: Value = serde_json::from_str(text)
        .map_err(|e| Error::invalid_request(format!("line is not valid JSON: {e}")))?;
    let Value::Object(members) = value else {
        return Err(Error::invalid_request("line is not a JSON object"));
    };
    let Some(Value::String(command)) = members.get("Command") else {
        return Err(Error::invalid_request("Command is absent or not a string"));
    };
    let params = Params(&members);
    match command.as_str() {
        "CreateQueue" => create_queue(&params),
        "SetQueueAttributes" => set_queue_attributes(&params),
        "GetQueueAttributes" => Ok(Request::GetQueueAttributes {
            queue: queue_name(&params)?,
        }),
        "ListQueues" => Ok(Request::ListQueues {
            prefix: params
                .optional("Prefix")
                .map(|v| string("Prefix", v).map(str::to_owned))
                .transpose()?,
        }),
        "DeleteQueue" => Ok(Request::DeleteQueue {
            queue: queue_name(&params)?,
        }),
        "PurgeQueue" => Ok(Request::PurgeQueue {
            queue: queue_name(&params)?,
        }),
        "Tick" => {
            let seconds = params.required("Seconds")?;
            Ok(Request::Tick {
                seconds: ranged("Seconds", seconds)?,
            })
        }
        "SendMessage" => send_message(&params),
        "ReceiveMessage" => receive_message(&params),
        "DeleteMessage" => {
            let (queue, handle) = queue_and_handle(&params)?;
            Ok(Request::DeleteMessage { queue, handle })
        }
        "ChangeMessageVisibility" => change_visibility(&params),
        other => Err(Error::invalid_request(format!("unknown command {other}"))),
    }
}

struct Params<'a>(&'a Map<String, Value>);

impl<'a> Params<'a> {
    /// A member whose value is `null` counts as absent.
    fn optional(&self, name: &str) -> Option<&'a Value> {
        self.0.get(name).filter(|v| !v.is_null())
    }

    fn required(&self, name: &str) -> Result<&'a Value> {
        self.optional(name).ok_or_else(|| Error::missing(name))
    }
}

fn string<'a>(name: &str, value: &'a Value) -> Result<&'a str> {
    value
        .as_str()
        .ok_or_else(|| Error::invalid_value(name, "must be a string"))
}

fn boolean(name: &str, value: &Value) -> Result<bool> {
    value
        .as_bool()
        .ok_or_else(|| Error::invalid_value(name, "must be true or false"))
}

fn ranged<T: Ranged>(name: &str, value: &Value) -> Result<T> {
    // With arbitrary precision enabled, only text written as a plain integer
    // converts, so 30.0 and 3e1 are rejected here like strings are.
    let number = value.as_number().and_then(serde_json::Number::as_i64);
    number.and_then(T::new).ok_or_else(|| {
        Error::invalid_value(
            name,
            format!("must be an integer from {} to {}", T::MIN, T::MAX),
        )
    })
}

fn token<T>(name: &str, value: &Value, parse: fn(&str) -> Option<T>) -> Result<T> {
    parse(string(name, value)?).ok_or_else(|| {
        Error::invalid_value(name, "must be 1 to 128 characters from A-Z a-z 0-9 _ -")
    })
}

fn queue_name(params: &Params<'_>) -> Result<String> {
    let value = params.required("QueueName")?;
    string("QueueName", value).map(str::to_owned)
}

fn create_queue(params: &Params<'_>) -> Result<Request> {
    let name = params.required("QueueName")?;
    let name = QueueName::parse(string("QueueName", name)?).ok_or_else(|| {
        Error::new(
            ErrorCode::QueueNameInvalid,
            "queue names are 1 to 80 characters from A-Z a-z 0-9 _ - with an optional .fifo suffix",
        )
    })?;
    let attributes = match params.optional("Attributes") {
        Some(value) => attributes(value)?,
        None => AttributeRequest::default(),
    };
    Ok(Request::CreateQueue { name, attributes })
}

fn set_queue_attributes(params: &Params<'_>) -> Result<Request> {
    let queue = params.required("QueueName")?;
    let attributes_value = params.required("Attributes")?;
    Ok(Request::SetQueueAttributes {
        queue: string("QueueName", queue)?.to_owned(),
        attributes: attributes(attributes_value)?,
    })
}

fn attributes(value: &Value) -> Result<AttributeRequest> {
    let Value::Object(members) = value else {
        return Err(Error::invalid_value("Attributes", "must be an object"));
    };
    let mut request = AttributeRequest::default();
    let mut dead_letter_queue = None;
    let mut max_receive_count = None;
    for (name, value) in members.iter().filter(|(_, v)| !v.is_null()) {
        match name.as_str() {
            "VisibilityTimeout" => request.visibility_timeout = Some(ranged(name, value)?),
            "DelaySeconds" => request.delay = Some(ranged(name, value)?),
            "RetentionSeconds" => {
                request.retention = Some(ranged::<RetentionSeconds>(name, value)?)
            }
            "Fifo" => request.fifo = Some(boolean(name, value)?),
            "ContentBasedDeduplication" => {
                request.content_based_deduplication = Some(boolean(name, value)?);
            }
            "DeadLetterQueue" => dead_letter_queue = Some(string(name, value)?.to_owned()),
            "MaxReceiveCount" => max_receive_count = Some(ranged::<MaxReceiveCount>(name, value)?),
            _ => return Err(Error::invalid_value(name, "is not a queue attribute")),
        }
    }
    request.dead_letter = match (dead_letter_queue, max_receive_count) {
        (None, None) => DeadLetterRequest::Absent,
        (Some(_), None) => DeadLetterRequest::QueueOnly,
        (None, Some(_)) => DeadLetterRequest::CountOnly,
        (Some(queue), Some(max_receive_count)) => DeadLetterRequest::Both {
            queue,
            max_receive_count,
        },
    };
    Ok(request)
}

fn send_message(params: &Params<'_>) -> Result<Request> {
    let queue = params.required("QueueName")?;
    let body = params.required("Body")?;
    let queue = string("QueueName", queue)?.to_owned();
    let body = Body::parse(string("Body", body)?)
        .ok_or_else(|| Error::invalid_value("Body", "must be 1 to 1024 bytes"))?;
    let delay = params
        .optional("DelaySeconds")
        .map(|v| ranged("DelaySeconds", v))
        .transpose()?;
    let group = params
        .optional("GroupId")
        .map(|v| token("GroupId", v, GroupId::parse))
        .transpose()?;
    let deduplication = params
        .optional("DeduplicationId")
        .map(|v| token("DeduplicationId", v, DeduplicationId::parse))
        .transpose()?
        .map(DedupKey::from);
    Ok(Request::SendMessage(Send {
        queue,
        body,
        delay,
        group,
        deduplication,
    }))
}

fn receive_message(params: &Params<'_>) -> Result<Request> {
    let queue = queue_name(params)?;
    let max_messages = params
        .optional("MaxMessages")
        .map(|v| ranged("MaxMessages", v))
        .transpose()?
        .unwrap_or(MaxMessages::DEFAULT);
    let visibility_timeout = params
        .optional("VisibilityTimeout")
        .map(|v| ranged("VisibilityTimeout", v))
        .transpose()?;
    Ok(Request::ReceiveMessage {
        queue,
        max_messages,
        visibility_timeout,
    })
}

fn queue_and_handle(params: &Params<'_>) -> Result<(String, HandleText)> {
    let queue = params.required("QueueName")?;
    let handle = params.required("ReceiptHandle")?;
    Ok((
        string("QueueName", queue)?.to_owned(),
        handle_text(string("ReceiptHandle", handle)?),
    ))
}

fn handle_text(text: &str) -> HandleText {
    ReceiptHandle::parse(text).map_or(HandleText::Malformed, HandleText::Canonical)
}

fn change_visibility(params: &Params<'_>) -> Result<Request> {
    let queue = params.required("QueueName")?;
    let handle = params.required("ReceiptHandle")?;
    let timeout = params.required("VisibilityTimeout")?;
    Ok(Request::ChangeMessageVisibility {
        queue: string("QueueName", queue)?.to_owned(),
        handle: handle_text(string("ReceiptHandle", handle)?),
        visibility_timeout: ranged("VisibilityTimeout", timeout)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code_of(line: &str) -> ErrorCode {
        parse(line.as_bytes())
            .expect_err("line must be rejected")
            .code()
    }

    #[test]
    fn malformed_lines_are_invalid_requests() {
        let lines = [
            "",
            "   ",
            "[]",
            "\"x\"",
            "{}",
            r#"{"Command": 5}"#,
            r#"{"Command": "Nope"}"#,
            r#"{"Command": "Tick"} trailing"#,
            r#"{"command": "Tick", "Seconds": 1}"#,
            "{\"Command\": \"Tick\"",
        ];
        for line in lines {
            assert_eq!(code_of(line), ErrorCode::InvalidRequest, "{line:?}");
        }
        assert_eq!(
            parse(b"{\"Command\": \"\xff\"}").unwrap_err().code(),
            ErrorCode::InvalidRequest
        );
    }

    #[test]
    fn required_parameters_are_checked_before_values() {
        let cases = [
            (r#"{"Command": "Tick"}"#, ErrorCode::MissingParameter),
            (
                r#"{"Command": "Tick", "Seconds": null}"#,
                ErrorCode::MissingParameter,
            ),
            (
                r#"{"Command": "SendMessage", "QueueName": 1}"#,
                ErrorCode::MissingParameter,
            ),
            (
                r#"{"Command": "SetQueueAttributes", "QueueName": 1}"#,
                ErrorCode::MissingParameter,
            ),
            (
                r#"{"Command": "DeleteMessage", "QueueName": 1}"#,
                ErrorCode::MissingParameter,
            ),
            (
                r#"{"Command": "Tick", "Seconds": "5"}"#,
                ErrorCode::InvalidParameterValue,
            ),
        ];
        for (line, code) in cases {
            assert_eq!(code_of(line), code, "{line}");
        }
    }

    #[test]
    fn integers_must_be_written_without_fraction_or_exponent() {
        let cases = [
            ("30", true),
            ("0", true),
            ("-0", true),
            ("30.0", false),
            ("3e1", false),
            ("\"30\"", false),
            ("true", false),
            ("-1", false),
            ("1209601", false),
            ("99999999999999999999", false),
        ];
        for (text, accepted) in cases {
            let line = format!(r#"{{"Command": "Tick", "Seconds": {text}}}"#);
            assert_eq!(parse(line.as_bytes()).is_ok(), accepted, "{text}");
        }
    }

    #[test]
    fn create_queue_checks_the_name_before_other_parameters() {
        let line = r#"{"Command": "CreateQueue", "QueueName": "bad name", "Attributes": 5}"#;
        assert_eq!(code_of(line), ErrorCode::QueueNameInvalid);
        let line = r#"{"Command": "CreateQueue", "QueueName": "ok", "Attributes": 5}"#;
        assert_eq!(code_of(line), ErrorCode::InvalidParameterValue);
        let line = r#"{"Command": "CreateQueue", "QueueName": "ok", "Attributes": {"Colour": 1}}"#;
        assert_eq!(code_of(line), ErrorCode::InvalidParameterValue);
    }

    #[test]
    fn null_attributes_are_absent() {
        let line = r#"{"Command": "CreateQueue", "QueueName": "q", "Attributes": {"Fifo": null, "Colour": null}}"#;
        let parsed = parse(line.as_bytes());
        assert_eq!(
            parsed,
            Ok(Request::CreateQueue {
                name: QueueName::parse("q").unwrap(),
                attributes: AttributeRequest::default(),
            })
        );
    }

    #[test]
    fn handles_are_kept_as_text_class() {
        let line = r#"{"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": "r-01"}"#;
        assert_eq!(
            parse(line.as_bytes()),
            Ok(Request::DeleteMessage {
                queue: "q".to_owned(),
                handle: HandleText::Malformed
            })
        );
    }
}
