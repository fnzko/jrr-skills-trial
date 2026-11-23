//! Parsing of command lines into typed requests.
//!
//! Checks run in the order of the specification: request shape, required parameters, then
//! parameter values. Rules that need broker state are applied by the broker afterwards.

use serde_json::{Map, Value};

use crate::attributes::AttributeUpdate;
use crate::error::{Error, ErrorCode, Result};
use crate::queue::NewMessage;
use crate::types::{
    Body, Bounded, DeduplicationId, GroupId, MaxMessages, QueueName, TickSeconds, VisibilityTimeout,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
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
    SendMessage {
        queue: String,
        message: NewMessage,
    },
    ReceiveMessage {
        queue: String,
        max_messages: MaxMessages,
        visibility_timeout: Option<VisibilityTimeout>,
    },
    DeleteMessage {
        queue: String,
        receipt_handle: String,
    },
    ChangeMessageVisibility {
        queue: String,
        receipt_handle: String,
        visibility_timeout: VisibilityTimeout,
    },
    Tick {
        by: TickSeconds,
    },
}

pub fn parse(line: &[u8]) -> Result<Request> {
    let text = std::str::from_utf8(line).map_err(|_| invalid_request("line is not valid UTF-8"))?;
    let value: Value = serde_json::from_str(text)
        .map_err(|err| invalid_request(format!("line is not valid JSON: {err}")))?;
    let Value::Object(members) = value else {
        return Err(invalid_request("line is not a JSON object"));
    };
    let mut params = Params(members);
    let Some(Value::String(command)) = params.0.remove("Command") else {
        return Err(invalid_request("`Command` is missing or not a string"));
    };
    match command.as_str() {
        "CreateQueue" => {
            params.require(&["QueueName"])?;
            // The name is checked before every other parameter of this command.
            let name = params.required_string("QueueName")?;
            let name = QueueName::new(name).ok_or_else(|| {
                Error::new(
                    ErrorCode::QueueNameInvalid,
                    "queue names are 1 to 80 characters from `A-Z a-z 0-9 _ -`, optionally followed by `.fifo` (at most 75 characters before it)",
                )
            })?;
            let attributes = params.optional_attributes()?;
            Ok(Request::CreateQueue { name, attributes })
        }
        "SetQueueAttributes" => {
            params.require(&["QueueName", "Attributes"])?;
            Ok(Request::SetQueueAttributes {
                queue: params.required_string("QueueName")?,
                attributes: params.optional_attributes()?,
            })
        }
        "GetQueueAttributes" => {
            params.require(&["QueueName"])?;
            Ok(Request::GetQueueAttributes {
                queue: params.required_string("QueueName")?,
            })
        }
        "ListQueues" => Ok(Request::ListQueues {
            prefix: params.optional_string("Prefix")?,
        }),
        "DeleteQueue" => {
            params.require(&["QueueName"])?;
            Ok(Request::DeleteQueue {
                queue: params.required_string("QueueName")?,
            })
        }
        "PurgeQueue" => {
            params.require(&["QueueName"])?;
            Ok(Request::PurgeQueue {
                queue: params.required_string("QueueName")?,
            })
        }
        "SendMessage" => {
            params.require(&["QueueName", "Body"])?;
            let queue = params.required_string("QueueName")?;
            let body = params.required_text("Body", Body::new, "1 to 1024 bytes of text")?;
            let delay = params.optional("DelaySeconds")?;
            let group_id = params.text("GroupId", GroupId::new, GROUP_TOKEN)?;
            let deduplication_id =
                params.text("DeduplicationId", DeduplicationId::new, GROUP_TOKEN)?;
            Ok(Request::SendMessage {
                queue,
                message: NewMessage {
                    body,
                    delay,
                    group_id,
                    deduplication_id,
                },
            })
        }
        "ReceiveMessage" => {
            params.require(&["QueueName"])?;
            Ok(Request::ReceiveMessage {
                queue: params.required_string("QueueName")?,
                max_messages: params.optional("MaxMessages")?.unwrap_or_default(),
                visibility_timeout: params.optional("VisibilityTimeout")?,
            })
        }
        "DeleteMessage" => {
            params.require(&["QueueName", "ReceiptHandle"])?;
            Ok(Request::DeleteMessage {
                queue: params.required_string("QueueName")?,
                receipt_handle: params.required_string("ReceiptHandle")?,
            })
        }
        "ChangeMessageVisibility" => {
            params.require(&["QueueName", "ReceiptHandle", "VisibilityTimeout"])?;
            Ok(Request::ChangeMessageVisibility {
                queue: params.required_string("QueueName")?,
                receipt_handle: params.required_string("ReceiptHandle")?,
                visibility_timeout: params.required("VisibilityTimeout")?,
            })
        }
        "Tick" => {
            params.require(&["Seconds"])?;
            Ok(Request::Tick {
                by: params.required("Seconds")?,
            })
        }
        other => Err(invalid_request(format!("unknown command `{other}`"))),
    }
}

const GROUP_TOKEN: &str = "1 to 128 characters from `A-Z a-z 0-9 _ -`";

fn invalid_request(message: impl Into<String>) -> Error {
    Error::new(ErrorCode::InvalidRequest, message)
}

fn missing(name: &str) -> Error {
    Error::new(ErrorCode::MissingParameter, format!("`{name}` is required"))
}

fn invalid_value(name: &str, expected: &str) -> Error {
    Error::new(
        ErrorCode::InvalidParameterValue,
        format!("`{name}` must be {expected}"),
    )
}

/// A number written with a fraction or an exponent is not an integer, and serde_json only
/// reports `as_i64` for numbers written as plain integers.
fn bounded_value<T: Bounded>(name: &str, value: &Value) -> Result<T> {
    value
        .as_i64()
        .and_then(|n| u32::try_from(n).ok())
        .and_then(T::new)
        .ok_or_else(|| {
            invalid_value(
                name,
                &format!("an integer from {} to {}", T::RANGE.start(), T::RANGE.end()),
            )
        })
}

fn string_value(name: &str, value: Value) -> Result<String> {
    match value {
        Value::String(text) => Ok(text),
        _ => Err(invalid_value(name, "a string")),
    }
}

fn bool_value(name: &str, value: &Value) -> Result<bool> {
    value
        .as_bool()
        .ok_or_else(|| invalid_value(name, "true or false"))
}

fn attribute_update(value: Value) -> Result<AttributeUpdate> {
    let Value::Object(members) = value else {
        return Err(invalid_value("Attributes", "an object"));
    };
    let mut update = AttributeUpdate::default();
    for (name, value) in members {
        if value.is_null() {
            continue;
        }
        match name.as_str() {
            "VisibilityTimeout" => update.visibility_timeout = Some(bounded_value(&name, &value)?),
            "DelaySeconds" => update.delay = Some(bounded_value(&name, &value)?),
            "RetentionSeconds" => update.retention = Some(bounded_value(&name, &value)?),
            "MaxReceiveCount" => update.max_receive_count = Some(bounded_value(&name, &value)?),
            "Fifo" => update.fifo = Some(bool_value(&name, &value)?),
            "ContentBasedDeduplication" => {
                update.content_based_deduplication = Some(bool_value(&name, &value)?);
            }
            "DeadLetterQueue" => update.dead_letter_queue = Some(string_value(&name, value)?),
            _ => {
                return Err(Error::new(
                    ErrorCode::InvalidParameterValue,
                    format!("`{name}` is not a queue attribute"),
                ));
            }
        }
    }
    Ok(update)
}

/// The members of a command. A member whose value is `null` counts as absent.
struct Params(Map<String, Value>);

impl Params {
    fn take(&mut self, name: &str) -> Option<Value> {
        self.0.remove(name).filter(|value| !value.is_null())
    }

    /// Fails with `MissingParameter` for the first of `names` that is absent.
    fn require(&self, names: &[&str]) -> Result<()> {
        match names
            .iter()
            .find(|name| self.0.get(**name).is_none_or(Value::is_null))
        {
            Some(name) => Err(missing(name)),
            None => Ok(()),
        }
    }

    fn required_string(&mut self, name: &str) -> Result<String> {
        let value = self.take(name).ok_or_else(|| missing(name))?;
        string_value(name, value)
    }

    fn optional_string(&mut self, name: &str) -> Result<Option<String>> {
        self.take(name)
            .map(|value| string_value(name, value))
            .transpose()
    }

    /// An absent `Attributes` member is the same as an empty one.
    fn optional_attributes(&mut self) -> Result<AttributeUpdate> {
        self.take("Attributes")
            .map(attribute_update)
            .transpose()
            .map(Option::unwrap_or_default)
    }

    /// A string parameter that must also satisfy the validity rule of its type.
    fn text<T>(
        &mut self,
        name: &str,
        build: fn(String) -> Option<T>,
        expected: &str,
    ) -> Result<Option<T>> {
        self.optional_string(name)?
            .map(|text| build(text).ok_or_else(|| invalid_value(name, expected)))
            .transpose()
    }

    fn required_text<T>(
        &mut self,
        name: &str,
        build: fn(String) -> Option<T>,
        expected: &str,
    ) -> Result<T> {
        self.text(name, build, expected)?
            .ok_or_else(|| missing(name))
    }

    fn optional<T: Bounded>(&mut self, name: &str) -> Result<Option<T>> {
        self.take(name)
            .map(|value| bounded_value(name, &value))
            .transpose()
    }

    fn required<T: Bounded>(&mut self, name: &str) -> Result<T> {
        let value = self.take(name).ok_or_else(|| missing(name))?;
        bounded_value(name, &value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tick(seconds: u32) -> Request {
        Request::Tick {
            by: TickSeconds::new(seconds).expect("test value is in range"),
        }
    }

    fn code(line: &str) -> ErrorCode {
        parse(line.as_bytes())
            .expect_err("line should be rejected")
            .code()
    }

    #[test]
    fn malformed_lines_are_invalid_requests() {
        for line in [
            "",
            "   ",
            "[]",
            "\"Tick\"",
            "{",
            "{} trailing",
            "{}",
            r#"{"Command": 1}"#,
            r#"{"Command": null}"#,
            r#"{"Command": "Nope"}"#,
            r#"{"command": "Tick", "Seconds": 1}"#,
        ] {
            assert_eq!(code(line), ErrorCode::InvalidRequest, "{line:?}");
        }
        assert_eq!(
            parse(b"{\"Command\": \"Tick\xff\"}"),
            Err(invalid_request("line is not valid UTF-8"))
        );
    }

    #[test]
    fn tick_reads_seconds_and_ignores_other_members() {
        assert_eq!(
            parse(br#"{"Command":"Tick","Seconds":5,"Extra":true}"#),
            Ok(tick(5))
        );
    }

    #[test]
    fn tick_without_usable_seconds_is_rejected() {
        for line in [
            r#"{"Command":"Tick"}"#,
            r#"{"Command":"Tick","Seconds":null}"#,
        ] {
            assert_eq!(code(line), ErrorCode::MissingParameter, "{line}");
        }
        for bad in ["-1", "1209601", "1.0", "1e1", "\"1\"", "true", "[]"] {
            let line = format!(r#"{{"Command":"Tick","Seconds":{bad}}}"#);
            assert_eq!(code(&line), ErrorCode::InvalidParameterValue, "{bad}");
        }
    }
}
