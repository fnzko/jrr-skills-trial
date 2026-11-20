//! Turns one input line into a typed [`Command`].
//!
//! Checks run in the order the spec fixes: request shape, required
//! parameters, then the type and range of each present parameter.
//! Everything that needs queue state is left to the broker.

use std::ops::RangeInclusive;

use serde_json::{Map, Value};

use crate::attributes::AttributeUpdate;
use crate::domain::{DeduplicationId, GroupId, InvalidQueueName, QueueName};
use crate::error::{Error, ErrorCode};

const VISIBILITY_TIMEOUT: RangeInclusive<u64> = 0..=43_200;
const DELAY_SECONDS: RangeInclusive<u64> = 0..=900;
const RETENTION_SECONDS: RangeInclusive<u64> = 60..=1_209_600;
const MAX_RECEIVE_COUNT: RangeInclusive<u64> = 1..=1000;
const TICK_SECONDS: RangeInclusive<u64> = 0..=1_209_600;
const MAX_MESSAGES: RangeInclusive<u64> = 1..=10;
const BODY_BYTES: RangeInclusive<usize> = 1..=1024;

/// A request whose parameters are well formed.
#[derive(Debug, Clone, PartialEq, Eq)]
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
        seconds: u64,
    },
    SendMessage {
        queue: String,
        body: String,
        delay_seconds: Option<u64>,
        group_id: Option<GroupId>,
        deduplication_id: Option<DeduplicationId>,
    },
    ReceiveMessage {
        queue: String,
        max_messages: usize,
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

/// Parses a line, without its line ending.
pub fn parse(line: &[u8]) -> Result<Command, Error> {
    let text = std::str::from_utf8(line)
        .map_err(|e| Error::invalid_request(format!("line is not valid UTF-8: {e}")))?;
    let value: Value = serde_json::from_str(text)
        .map_err(|e| Error::invalid_request(format!("line is not valid JSON: {e}")))?;
    let Value::Object(members) = value else {
        return Err(Error::invalid_request("line is not a JSON object"));
    };
    let params = Params(members);
    let Some(Value::String(name)) = params.get("Command") else {
        return Err(Error::invalid_request("Command must be a string"));
    };
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
        "Tick" => tick(&params),
        "SendMessage" => send_message(&params),
        "ReceiveMessage" => receive_message(&params),
        "DeleteMessage" => delete_message(&params),
        "ChangeMessageVisibility" => change_message_visibility(&params),
        unknown => Err(Error::invalid_request(format!(
            "unknown command {unknown:?}"
        ))),
    }
}

/// The members of a request. A member that is `null` counts as absent.
struct Params(Map<String, Value>);

impl Params {
    fn get(&self, name: &str) -> Option<&Value> {
        self.0.get(name).filter(|value| !value.is_null())
    }

    fn required(&self, name: &str) -> Result<&Value, Error> {
        self.get(name).ok_or_else(|| Error::missing(name))
    }

    fn optional<'a, T>(
        &'a self,
        name: &str,
        convert: impl FnOnce(&str, &'a Value) -> Result<T, Error>,
    ) -> Result<Option<T>, Error> {
        self.get(name).map(|value| convert(name, value)).transpose()
    }
}

fn string<'a>(name: &str, value: &'a Value) -> Result<&'a str, Error> {
    value
        .as_str()
        .ok_or_else(|| Error::invalid_value(name, "must be a string"))
}

fn boolean(name: &str, value: &Value) -> Result<bool, Error> {
    value
        .as_bool()
        .ok_or_else(|| Error::invalid_value(name, "must be a boolean"))
}

fn integer(name: &str, value: &Value, range: RangeInclusive<u64>) -> Result<u64, Error> {
    value
        .as_u64()
        .filter(|number| range.contains(number))
        .ok_or_else(|| {
            Error::invalid_value(
                name,
                format!(
                    "must be an integer from {} to {}",
                    range.start(),
                    range.end()
                ),
            )
        })
}

fn attributes(name: &str, value: &Value) -> Result<AttributeUpdate, Error> {
    let Value::Object(members) = value else {
        return Err(Error::invalid_value(name, "must be an object"));
    };
    let mut update = AttributeUpdate::default();
    for (key, value) in members.iter().filter(|(_, value)| !value.is_null()) {
        match key.as_str() {
            "VisibilityTimeout" => {
                update.visibility_timeout = Some(integer(key, value, VISIBILITY_TIMEOUT)?);
            }
            "DelaySeconds" => update.delay_seconds = Some(integer(key, value, DELAY_SECONDS)?),
            "RetentionSeconds" => {
                update.retention_seconds = Some(integer(key, value, RETENTION_SECONDS)?);
            }
            "Fifo" => update.fifo = Some(boolean(key, value)?),
            "ContentBasedDeduplication" => {
                update.content_based_deduplication = Some(boolean(key, value)?);
            }
            "DeadLetterQueue" => update.dead_letter_queue = Some(string(key, value)?.to_owned()),
            "MaxReceiveCount" => {
                update.max_receive_count = Some(integer(key, value, MAX_RECEIVE_COUNT)?);
            }
            _ => {
                return Err(Error::invalid_value(
                    &format!("{name}.{key}"),
                    "is not a queue attribute",
                ));
            }
        }
    }
    Ok(update)
}

/// The `QueueName` of every command but `CreateQueue`: any string will do,
/// whether a queue has that name is the broker's question.
fn queue_name(params: &Params) -> Result<String, Error> {
    let value = params.required("QueueName")?;
    string("QueueName", value).map(str::to_owned)
}

fn create_queue(params: &Params) -> Result<Command, Error> {
    let name = params.required("QueueName")?;
    let name = string("QueueName", name)?
        .parse()
        .map_err(|e: InvalidQueueName| Error::new(ErrorCode::QueueNameInvalid, e.to_string()))?;
    let attributes = params
        .optional("Attributes", attributes)?
        .unwrap_or_default();
    Ok(Command::CreateQueue { name, attributes })
}

fn set_queue_attributes(params: &Params) -> Result<Command, Error> {
    let queue = params.required("QueueName")?;
    let update = params.required("Attributes")?;
    Ok(Command::SetQueueAttributes {
        queue: string("QueueName", queue)?.to_owned(),
        attributes: attributes("Attributes", update)?,
    })
}

fn tick(params: &Params) -> Result<Command, Error> {
    let seconds = params.required("Seconds")?;
    Ok(Command::Tick {
        seconds: integer("Seconds", seconds, TICK_SECONDS)?,
    })
}

fn client_id<T: std::str::FromStr<Err = crate::domain::InvalidId>>(
    name: &str,
    value: &Value,
) -> Result<T, Error> {
    string(name, value)?
        .parse()
        .map_err(|e| Error::invalid_value(name, e))
}

fn send_message(params: &Params) -> Result<Command, Error> {
    let queue = params.required("QueueName")?;
    let body = params.required("Body")?;
    let queue = string("QueueName", queue)?.to_owned();
    let body = string("Body", body)?;
    if !BODY_BYTES.contains(&body.len()) {
        return Err(Error::invalid_value(
            "Body",
            format!(
                "must be {} to {} bytes",
                BODY_BYTES.start(),
                BODY_BYTES.end()
            ),
        ));
    }
    Ok(Command::SendMessage {
        queue,
        body: body.to_owned(),
        delay_seconds: params.optional("DelaySeconds", |n, v| integer(n, v, DELAY_SECONDS))?,
        group_id: params.optional("GroupId", client_id)?,
        deduplication_id: params.optional("DeduplicationId", client_id)?,
    })
}

fn receive_message(params: &Params) -> Result<Command, Error> {
    let queue = params.required("QueueName")?;
    let queue = string("QueueName", queue)?.to_owned();
    let max_messages = params
        .optional("MaxMessages", |n, v| integer(n, v, MAX_MESSAGES))?
        .unwrap_or(1);
    Ok(Command::ReceiveMessage {
        queue,
        max_messages: usize::try_from(max_messages)
            .expect("MaxMessages is at most 10, which fits in usize"),
        visibility_timeout: params.optional("VisibilityTimeout", |n, v| {
            integer(n, v, VISIBILITY_TIMEOUT)
        })?,
    })
}

fn delete_message(params: &Params) -> Result<Command, Error> {
    let queue = params.required("QueueName")?;
    let handle = params.required("ReceiptHandle")?;
    Ok(Command::DeleteMessage {
        queue: string("QueueName", queue)?.to_owned(),
        receipt_handle: string("ReceiptHandle", handle)?.to_owned(),
    })
}

fn change_message_visibility(params: &Params) -> Result<Command, Error> {
    let queue = params.required("QueueName")?;
    let handle = params.required("ReceiptHandle")?;
    let timeout = params.required("VisibilityTimeout")?;
    Ok(Command::ChangeMessageVisibility {
        queue: string("QueueName", queue)?.to_owned(),
        receipt_handle: string("ReceiptHandle", handle)?.to_owned(),
        visibility_timeout: integer("VisibilityTimeout", timeout, VISIBILITY_TIMEOUT)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code(line: &str) -> ErrorCode {
        parse(line.as_bytes()).unwrap_err().code()
    }

    #[test]
    fn parses_a_command_with_its_parameters() {
        let command = parse(br#"{"Command":"Tick","Seconds":5,"Extra":[1]}"#).unwrap();
        assert_eq!(command, Command::Tick { seconds: 5 });
    }

    #[test]
    fn request_shape_errors_are_invalid_request() {
        let lines = [
            "",
            "   ",
            "[]",
            "\"Tick\"",
            "{",
            "{} trailing",
            "{}",
            r#"{"Command":null}"#,
            r#"{"Command":7}"#,
            r#"{"Command":"tick","Seconds":1}"#,
            r#"{"command":"Tick","Seconds":1}"#,
            r#"{"Command":"Nope"}"#,
        ];
        for line in lines {
            assert_eq!(code(line), ErrorCode::InvalidRequest, "line {line:?}");
        }
        assert_eq!(
            parse(b"{\"Command\":\"Tick\",\"Seconds\":\xff}")
                .unwrap_err()
                .code(),
            ErrorCode::InvalidRequest
        );
    }

    #[test]
    fn null_counts_as_absent() {
        assert_eq!(
            code(r#"{"Command":"Tick","Seconds":null}"#),
            ErrorCode::MissingParameter
        );
        let command = parse(br#"{"Command":"ListQueues","Prefix":null}"#).unwrap();
        assert_eq!(command, Command::ListQueues { prefix: None });
    }

    #[test]
    fn integers_are_written_without_fraction_or_exponent() {
        for text in ["30.0", "3e1", "\"30\"", "-1", "true", "[30]", "1209601"] {
            let line = format!(r#"{{"Command":"Tick","Seconds":{text}}}"#);
            assert_eq!(code(&line), ErrorCode::InvalidParameterValue, "line {line}");
        }
        let command = parse(br#"{"Command":"Tick","Seconds":1209600}"#).unwrap();
        assert_eq!(command, Command::Tick { seconds: 1_209_600 });
    }

    #[test]
    fn missing_parameters_win_over_invalid_values() {
        let line = r#"{"Command":"SendMessage","QueueName":5}"#;
        assert_eq!(code(line), ErrorCode::MissingParameter);
    }

    #[test]
    fn create_queue_checks_the_name_before_other_parameters() {
        let line = r#"{"Command":"CreateQueue","QueueName":"a b","Attributes":5}"#;
        assert_eq!(code(line), ErrorCode::QueueNameInvalid);
        let line = r#"{"Command":"CreateQueue","QueueName":5,"Attributes":5}"#;
        assert_eq!(code(line), ErrorCode::InvalidParameterValue);
        let line = r#"{"Command":"CreateQueue","QueueName":"ok","Attributes":5}"#;
        assert_eq!(code(line), ErrorCode::InvalidParameterValue);
    }

    #[test]
    fn other_commands_accept_any_queue_name_string() {
        let command =
            parse(br#"{"Command":"PurgeQueue","QueueName":"not a valid name!"}"#).unwrap();
        assert_eq!(
            command,
            Command::PurgeQueue {
                queue: "not a valid name!".to_owned()
            }
        );
    }

    #[test]
    fn attributes_are_checked_one_by_one() {
        let bad = [
            r#"{"VisibilityTimeout":43201}"#,
            r#"{"DelaySeconds":-1}"#,
            r#"{"RetentionSeconds":59}"#,
            r#"{"Fifo":"true"}"#,
            r#"{"ContentBasedDeduplication":1}"#,
            r#"{"DeadLetterQueue":3,"MaxReceiveCount":1}"#,
            r#"{"MaxReceiveCount":0}"#,
            r#"{"Unknown":1}"#,
        ];
        for attributes in bad {
            let line =
                format!(r#"{{"Command":"CreateQueue","QueueName":"q","Attributes":{attributes}}}"#);
            assert_eq!(code(&line), ErrorCode::InvalidParameterValue, "line {line}");
        }
    }

    #[test]
    fn null_attributes_are_absent() {
        let command = parse(
            br#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"Fifo":null,"Nope":null,"DelaySeconds":4}}"#,
        )
        .unwrap();
        let expected = AttributeUpdate {
            delay_seconds: Some(4),
            ..AttributeUpdate::default()
        };
        assert_eq!(
            command,
            Command::SetQueueAttributes {
                queue: "q".to_owned(),
                attributes: expected
            }
        );
    }

    #[test]
    fn send_message_limits_the_body_in_bytes() {
        let send =
            |body: &str| format!(r#"{{"Command":"SendMessage","QueueName":"q","Body":"{body}"}}"#);
        assert_eq!(code(&send("")), ErrorCode::InvalidParameterValue);
        assert_eq!(
            code(&send(&"a".repeat(1025))),
            ErrorCode::InvalidParameterValue
        );
        // 513 two-byte characters are 1026 bytes.
        assert_eq!(
            code(&send(&"é".repeat(513))),
            ErrorCode::InvalidParameterValue
        );
        assert!(parse(send(&"a".repeat(1024)).as_bytes()).is_ok());
        assert!(parse(send(&"é".repeat(512)).as_bytes()).is_ok());
    }

    #[test]
    fn send_message_ids_are_checked_even_for_unused_members() {
        let line = r#"{"Command":"SendMessage","QueueName":"q","Body":"b","GroupId":"a b"}"#;
        assert_eq!(code(line), ErrorCode::InvalidParameterValue);
        let line = r#"{"Command":"SendMessage","QueueName":"q","Body":"b","DeduplicationId":""}"#;
        assert_eq!(code(line), ErrorCode::InvalidParameterValue);
    }

    #[test]
    fn receive_message_defaults_and_ranges() {
        let command = parse(br#"{"Command":"ReceiveMessage","QueueName":"q"}"#).unwrap();
        assert_eq!(
            command,
            Command::ReceiveMessage {
                queue: "q".to_owned(),
                max_messages: 1,
                visibility_timeout: None
            }
        );
        for member in [
            r#""MaxMessages":0"#,
            r#""MaxMessages":11"#,
            r#""VisibilityTimeout":43201"#,
        ] {
            let line = format!(r#"{{"Command":"ReceiveMessage","QueueName":"q",{member}}}"#);
            assert_eq!(code(&line), ErrorCode::InvalidParameterValue, "line {line}");
        }
    }

    #[test]
    fn change_message_visibility_requires_its_timeout() {
        let line = r#"{"Command":"ChangeMessageVisibility","QueueName":"q","ReceiptHandle":"r-1"}"#;
        assert_eq!(code(line), ErrorCode::MissingParameter);
    }
}
