//! Turns a line of input into a typed command, applying the parameter checks of
//! the spec in order: shape, required parameters, then values.

use std::ops::RangeInclusive;

use serde_json::{Map, Value};

use crate::attributes::AttributePatch;
use crate::error::{Error, ErrorCode};
use crate::types::{Body, GroupId, QueueName, Seconds, validate_token};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    CreateQueue {
        name: QueueName,
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
        seconds: Seconds,
    },
    SendMessage {
        queue: String,
        body: Body,
        delay: Option<Seconds>,
        group: Option<GroupId>,
        deduplication_id: Option<String>,
    },
    ReceiveMessage {
        queue: String,
        max_messages: u32,
        visibility_timeout: Option<Seconds>,
    },
    DeleteMessage {
        queue: String,
        receipt_handle: String,
    },
    ChangeMessageVisibility {
        queue: String,
        receipt_handle: String,
        visibility_timeout: Seconds,
    },
}

/// The members of a command object; `null` counts as absent.
struct Fields<'a>(&'a Map<String, Value>);

impl<'a> Fields<'a> {
    fn get(&self, name: &str) -> Option<&'a Value> {
        self.0.get(name).filter(|value| !value.is_null())
    }

    fn require(&self, name: &str) -> Result<&'a Value, Error> {
        self.get(name).ok_or_else(|| Error::missing(name))
    }

    fn optional_str(&self, name: &str) -> Result<Option<&'a str>, Error> {
        self.get(name).map(|value| as_str(value, name)).transpose()
    }

    fn optional_seconds(
        &self,
        name: &str,
        range: RangeInclusive<u32>,
    ) -> Result<Option<Seconds>, Error> {
        self.get(name)
            .map(|value| as_int(value, name, range).map(Seconds::from))
            .transpose()
    }
}

fn invalid_request(message: &str) -> Error {
    Error::new(ErrorCode::InvalidRequest, message)
}

fn as_str<'a>(value: &'a Value, name: &str) -> Result<&'a str, Error> {
    value
        .as_str()
        .ok_or_else(|| Error::invalid_value(format!("{name} must be a string")))
}

fn as_bool(value: &Value, name: &str) -> Result<bool, Error> {
    value
        .as_bool()
        .ok_or_else(|| Error::invalid_value(format!("{name} must be a boolean")))
}

/// An integer written without fraction or exponent, within `range`.
fn as_int(value: &Value, name: &str, range: RangeInclusive<u32>) -> Result<u32, Error> {
    value
        .as_u64()
        .and_then(|n| u32::try_from(n).ok())
        .filter(|n| range.contains(n))
        .ok_or_else(|| {
            Error::invalid_value(format!(
                "{name} must be an integer from {} to {}",
                range.start(),
                range.end()
            ))
        })
}

fn parse_attributes(value: &Value) -> Result<AttributePatch, Error> {
    let Value::Object(members) = value else {
        return Err(Error::invalid_value("Attributes must be an object"));
    };
    let mut patch = AttributePatch::default();
    for (name, value) in members {
        let value = Some(value).filter(|value| !value.is_null());
        match name.as_str() {
            "VisibilityTimeout" => {
                patch.visibility_timeout = seconds(value, name, 0..=43_200)?;
            }
            "DelaySeconds" => patch.delay_seconds = seconds(value, name, 0..=900)?,
            "RetentionSeconds" => {
                patch.retention_seconds = seconds(value, name, 60..=1_209_600)?;
            }
            "Fifo" => patch.fifo = value.map(|v| as_bool(v, name)).transpose()?,
            "ContentBasedDeduplication" => {
                patch.content_based_deduplication = value.map(|v| as_bool(v, name)).transpose()?;
            }
            "DeadLetterQueue" => {
                patch.dead_letter_queue = value
                    .map(|v| as_str(v, name).map(str::to_owned))
                    .transpose()?;
            }
            "MaxReceiveCount" => {
                patch.max_receive_count = value.map(|v| as_int(v, name, 1..=1000)).transpose()?;
            }
            _ => return Err(Error::invalid_value(format!("unknown attribute {name}"))),
        }
    }
    Ok(patch)
}

fn seconds(
    value: Option<&Value>,
    name: &str,
    range: RangeInclusive<u32>,
) -> Result<Option<Seconds>, Error> {
    value
        .map(|v| as_int(v, name, range).map(Seconds::from))
        .transpose()
}

/// Parses one input line.
pub fn parse(line: &[u8]) -> Result<Command, Error> {
    let text = std::str::from_utf8(line).map_err(|_| invalid_request("line is not valid UTF-8"))?;
    let value: Value = serde_json::from_str(text)
        .map_err(|e| invalid_request(&format!("line is not valid JSON: {e}")))?;
    let Value::Object(members) = value else {
        return Err(invalid_request("line is not a JSON object"));
    };
    let fields = Fields(&members);
    let Some(Value::String(name)) = fields.get("Command") else {
        return Err(invalid_request("Command must be a string"));
    };
    match name.as_str() {
        "CreateQueue" => create_queue(&fields),
        "SetQueueAttributes" => {
            let queue = fields.require("QueueName")?;
            let attributes = fields.require("Attributes")?;
            Ok(Command::SetQueueAttributes {
                queue: as_str(queue, "QueueName")?.to_owned(),
                attributes: parse_attributes(attributes)?,
            })
        }
        "GetQueueAttributes" => Ok(Command::GetQueueAttributes {
            queue: queue_name(&fields)?,
        }),
        "ListQueues" => Ok(Command::ListQueues {
            prefix: fields.optional_str("Prefix")?.map(str::to_owned),
        }),
        "DeleteQueue" => Ok(Command::DeleteQueue {
            queue: queue_name(&fields)?,
        }),
        "PurgeQueue" => Ok(Command::PurgeQueue {
            queue: queue_name(&fields)?,
        }),
        "Tick" => {
            let seconds = fields.require("Seconds")?;
            Ok(Command::Tick {
                seconds: as_int(seconds, "Seconds", 0..=1_209_600)?.into(),
            })
        }
        "SendMessage" => send_message(&fields),
        "ReceiveMessage" => receive_message(&fields),
        "DeleteMessage" => {
            let (queue, receipt_handle) = queue_and_handle(&fields)?;
            Ok(Command::DeleteMessage {
                queue,
                receipt_handle,
            })
        }
        "ChangeMessageVisibility" => {
            let queue = fields.require("QueueName")?;
            let handle = fields.require("ReceiptHandle")?;
            let timeout = fields.require("VisibilityTimeout")?;
            Ok(Command::ChangeMessageVisibility {
                queue: as_str(queue, "QueueName")?.to_owned(),
                receipt_handle: as_str(handle, "ReceiptHandle")?.to_owned(),
                visibility_timeout: as_int(timeout, "VisibilityTimeout", 0..=43_200)?.into(),
            })
        }
        other => Err(invalid_request(&format!("unknown command {other:?}"))),
    }
}

fn queue_name(fields: &Fields<'_>) -> Result<String, Error> {
    as_str(fields.require("QueueName")?, "QueueName").map(str::to_owned)
}

fn queue_and_handle(fields: &Fields<'_>) -> Result<(String, String), Error> {
    let queue = fields.require("QueueName")?;
    let handle = fields.require("ReceiptHandle")?;
    Ok((
        as_str(queue, "QueueName")?.to_owned(),
        as_str(handle, "ReceiptHandle")?.to_owned(),
    ))
}

fn create_queue(fields: &Fields<'_>) -> Result<Command, Error> {
    let name = fields.require("QueueName")?;
    let name = QueueName::parse(as_str(name, "QueueName")?)?;
    let attributes = fields
        .get("Attributes")
        .map(parse_attributes)
        .transpose()?
        .unwrap_or_default();
    Ok(Command::CreateQueue { name, attributes })
}

fn send_message(fields: &Fields<'_>) -> Result<Command, Error> {
    let queue = fields.require("QueueName")?;
    let body = fields.require("Body")?;
    Ok(Command::SendMessage {
        queue: as_str(queue, "QueueName")?.to_owned(),
        body: Body::parse(as_str(body, "Body")?)?,
        delay: fields.optional_seconds("DelaySeconds", 0..=900)?,
        group: fields
            .optional_str("GroupId")?
            .map(GroupId::parse)
            .transpose()?,
        deduplication_id: fields
            .optional_str("DeduplicationId")?
            .map(|id| validate_token(id, "DeduplicationId").map(str::to_owned))
            .transpose()?,
    })
}

fn receive_message(fields: &Fields<'_>) -> Result<Command, Error> {
    let queue = fields.require("QueueName")?;
    Ok(Command::ReceiveMessage {
        queue: as_str(queue, "QueueName")?.to_owned(),
        max_messages: fields
            .get("MaxMessages")
            .map(|value| as_int(value, "MaxMessages", 1..=10))
            .transpose()?
            .unwrap_or(1),
        visibility_timeout: fields.optional_seconds("VisibilityTimeout", 0..=43_200)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code(line: &str) -> ErrorCode {
        parse(line.as_bytes())
            .expect_err("line should be rejected")
            .code
    }

    #[test]
    fn parses_a_command_ignoring_unknown_members() {
        let command = parse(br#"{"Command":"PurgeQueue","QueueName":"jobs","Extra":1}"#);
        assert_eq!(
            command,
            Ok(Command::PurgeQueue {
                queue: "jobs".into()
            })
        );
    }

    #[test]
    fn null_parameters_are_absent() {
        assert_eq!(
            parse(br#"{"Command":"ListQueues","Prefix":null}"#),
            Ok(Command::ListQueues { prefix: None })
        );
        assert_eq!(
            code(r#"{"Command":"PurgeQueue","QueueName":null}"#),
            ErrorCode::MissingParameter
        );
    }

    #[test]
    fn bad_shapes_are_invalid_requests() {
        for line in [
            "",
            "[]",
            "not json",
            r#"{"Command":1}"#,
            r#"{"Command":null}"#,
            r#"{"QueueName":"a"}"#,
            r#"{"Command":"Nope"}"#,
            r#"{"command":"Tick","Seconds":1}"#,
            r#"{"Command":"Tick","Seconds":1} x"#,
        ] {
            assert_eq!(code(line), ErrorCode::InvalidRequest, "{line}");
        }
        assert_eq!(
            parse(b"{\"Command\":\"Tick\",\"Seconds\":\xff}")
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }

    #[test]
    fn integers_must_be_plain_and_in_range() {
        for seconds in ["30.0", "3e1", "\"30\"", "-1", "1209601", "true"] {
            let line = format!(r#"{{"Command":"Tick","Seconds":{seconds}}}"#);
            assert_eq!(code(&line), ErrorCode::InvalidParameterValue, "{seconds}");
        }
        assert_eq!(
            parse(br#"{"Command":"Tick","Seconds":1209600}"#),
            Ok(Command::Tick {
                seconds: Seconds(1_209_600)
            })
        );
    }

    #[test]
    fn missing_parameters_win_over_bad_values() {
        assert_eq!(
            code(r#"{"Command":"ChangeMessageVisibility","QueueName":1,"ReceiptHandle":"r-1"}"#),
            ErrorCode::MissingParameter
        );
        assert_eq!(
            code(r#"{"Command":"SetQueueAttributes","QueueName":"a"}"#),
            ErrorCode::MissingParameter
        );
    }

    #[test]
    fn create_queue_checks_the_name_before_other_values() {
        assert_eq!(
            code(r#"{"Command":"CreateQueue","QueueName":"a b","Attributes":5}"#),
            ErrorCode::QueueNameInvalid
        );
        assert_eq!(
            code(r#"{"Command":"CreateQueue","QueueName":5}"#),
            ErrorCode::InvalidParameterValue
        );
        assert_eq!(
            code(r#"{"Command":"CreateQueue","QueueName":"a","Attributes":{"Bogus":1}}"#),
            ErrorCode::InvalidParameterValue
        );
    }

    #[test]
    fn attribute_values_are_checked_and_nulls_ignored() {
        let command = parse(
            br#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"DelaySeconds":null,"MaxReceiveCount":7}}"#,
        );
        assert_eq!(
            command,
            Ok(Command::CreateQueue {
                name: QueueName::parse("q").expect("valid name"),
                attributes: AttributePatch {
                    max_receive_count: Some(7),
                    ..AttributePatch::default()
                },
            })
        );
        for attribute in [
            r#""DelaySeconds":901"#,
            r#""RetentionSeconds":59"#,
            r#""MaxReceiveCount":0"#,
            r#""Fifo":"true""#,
            r#""DeadLetterQueue":3"#,
        ] {
            let line = format!(
                r#"{{"Command":"CreateQueue","QueueName":"q","Attributes":{{{attribute}}}}}"#
            );
            assert_eq!(code(&line), ErrorCode::InvalidParameterValue, "{attribute}");
        }
    }

    #[test]
    fn send_message_checks_body_and_ids() {
        for extra in [
            r#""Body":"""#,
            r#""Body":5"#,
            r#""Body":"x","GroupId":"has space""#,
            r#""Body":"x","DeduplicationId":"""#,
            r#""Body":"x","DelaySeconds":901"#,
        ] {
            let line = format!(r#"{{"Command":"SendMessage","QueueName":"q",{extra}}}"#);
            assert_eq!(code(&line), ErrorCode::InvalidParameterValue, "{extra}");
        }
    }
}
