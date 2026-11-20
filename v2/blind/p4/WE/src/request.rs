//! Parsing of request lines into typed commands.
//!
//! Parsing performs the first three checks of the spec's error order: request
//! shape, then presence of every required parameter, then the value of each
//! present parameter. Checks that need broker state happen in the broker.

use std::ops::RangeInclusive;

use serde_json::{Map, Value};

use crate::error::Error;
use crate::types::{
    Body, DeadLetterRef, DeduplicationId, GroupId, InvalidValue, MaxReceiveCount, QueueName,
    Seconds,
};

const VISIBILITY_TIMEOUT_RANGE: RangeInclusive<u32> = 0..=43_200;
const DELAY_RANGE: RangeInclusive<u32> = 0..=900;
const RETENTION_RANGE: RangeInclusive<u32> = 60..=1_209_600;
const TICK_RANGE: RangeInclusive<u32> = 0..=1_209_600;
const MAX_MESSAGES_RANGE: RangeInclusive<u32> = 1..=10;

/// One parsed command. Queue names other than in `CreateQueue` are kept as
/// given, because any string is looked up and an unknown one is
/// `QueueDoesNotExist` whatever it looks like.
#[derive(Debug, PartialEq, Eq)]
pub enum Request {
    CreateQueue {
        queue: QueueName,
        attributes: AttributeChanges,
    },
    SetQueueAttributes {
        queue: String,
        attributes: AttributeChanges,
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
    SendMessage(SendMessage),
    ReceiveMessage(ReceiveMessage),
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

#[derive(Debug, PartialEq, Eq)]
pub struct SendMessage {
    pub queue: String,
    pub body: Body,
    pub delay: Option<Seconds>,
    pub group: Option<GroupId>,
    pub deduplication_id: Option<DeduplicationId>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ReceiveMessage {
    pub queue: String,
    /// From 1 to 10.
    pub max_messages: usize,
    pub visibility_timeout: Option<Seconds>,
}

/// The attributes named in an `Attributes` object, each `None` when absent.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct AttributeChanges {
    pub visibility_timeout: Option<Seconds>,
    pub delay: Option<Seconds>,
    pub retention: Option<Seconds>,
    pub fifo: Option<bool>,
    pub content_based_deduplication: Option<bool>,
    dead_letter_queue: Option<String>,
    max_receive_count: Option<MaxReceiveCount>,
}

impl AttributeChanges {
    /// The requested dead-letter configuration, which must name both of its
    /// attributes or neither.
    pub fn dead_letter(&self) -> Result<Option<DeadLetterRef<'_>>, Error> {
        match (&self.dead_letter_queue, self.max_receive_count) {
            (Some(queue), Some(max_receive_count)) => Ok(Some(DeadLetterRef {
                queue,
                max_receive_count,
            })),
            (None, None) => Ok(None),
            (Some(_), None) | (None, Some(_)) => Err(Error::InvalidParameterValue {
                parameter: "Attributes",
                rule: "must give DeadLetterQueue and MaxReceiveCount together",
            }),
        }
    }
}

/// Parses one input line, without its terminating newline.
pub fn parse(line: &[u8]) -> Result<Request, Error> {
    let text = std::str::from_utf8(line).map_err(|_| Error::NotUtf8)?;
    let Ok(Value::Object(members)) = serde_json::from_str(text) else {
        return Err(Error::NotAnObject);
    };
    let mut params = Params(members);
    let Some(Value::String(command)) = params.0.remove("Command") else {
        return Err(Error::NoCommand);
    };
    match command.as_str() {
        "CreateQueue" => {
            let queue = params.required("QueueName")?;
            let queue = queue_name(queue)?;
            let attributes = params.optional("Attributes").map(attributes).transpose()?;
            Ok(Request::CreateQueue {
                queue,
                attributes: attributes.unwrap_or_default(),
            })
        }
        "SetQueueAttributes" => {
            let queue = params.required("QueueName")?;
            let changes = params.required("Attributes")?;
            Ok(Request::SetQueueAttributes {
                queue: string(queue, "QueueName")?,
                attributes: attributes(changes)?,
            })
        }
        "GetQueueAttributes" => Ok(Request::GetQueueAttributes {
            queue: params.queue_name()?,
        }),
        "ListQueues" => Ok(Request::ListQueues {
            prefix: params
                .optional("Prefix")
                .map(|v| string(v, "Prefix"))
                .transpose()?,
        }),
        "DeleteQueue" => Ok(Request::DeleteQueue {
            queue: params.queue_name()?,
        }),
        "PurgeQueue" => Ok(Request::PurgeQueue {
            queue: params.queue_name()?,
        }),
        "Tick" => {
            let seconds = params.required("Seconds")?;
            Ok(Request::Tick {
                seconds: seconds_in(seconds, "Seconds", TICK_RANGE)?,
            })
        }
        "SendMessage" => {
            let queue = params.required("QueueName")?;
            let body = params.required("Body")?;
            Ok(Request::SendMessage(SendMessage {
                queue: string(queue, "QueueName")?,
                body: checked(string(body, "Body")?, "Body", Body::parse)?,
                delay: params
                    .optional("DelaySeconds")
                    .map(|v| seconds_in(v, "DelaySeconds", DELAY_RANGE))
                    .transpose()?,
                group: params
                    .optional("GroupId")
                    .map(|v| checked(string(v, "GroupId")?, "GroupId", GroupId::parse))
                    .transpose()?,
                deduplication_id: params
                    .optional("DeduplicationId")
                    .map(|v| {
                        checked(
                            string(v, "DeduplicationId")?,
                            "DeduplicationId",
                            DeduplicationId::parse,
                        )
                    })
                    .transpose()?,
            }))
        }
        "ReceiveMessage" => {
            let queue = params.required("QueueName")?;
            let max_messages = match params.optional("MaxMessages") {
                Some(value) => integer_in(value, "MaxMessages", MAX_MESSAGES_RANGE)?,
                None => 1,
            };
            Ok(Request::ReceiveMessage(ReceiveMessage {
                queue: string(queue, "QueueName")?,
                max_messages: usize::try_from(max_messages)
                    .expect("MaxMessages is at most 10, which fits any usize"),
                visibility_timeout: params
                    .optional("VisibilityTimeout")
                    .map(|v| seconds_in(v, "VisibilityTimeout", VISIBILITY_TIMEOUT_RANGE))
                    .transpose()?,
            }))
        }
        "DeleteMessage" => {
            let queue = params.required("QueueName")?;
            let receipt_handle = params.required("ReceiptHandle")?;
            Ok(Request::DeleteMessage {
                queue: string(queue, "QueueName")?,
                receipt_handle: string(receipt_handle, "ReceiptHandle")?,
            })
        }
        "ChangeMessageVisibility" => {
            let queue = params.required("QueueName")?;
            let receipt_handle = params.required("ReceiptHandle")?;
            let timeout = params.required("VisibilityTimeout")?;
            Ok(Request::ChangeMessageVisibility {
                queue: string(queue, "QueueName")?,
                receipt_handle: string(receipt_handle, "ReceiptHandle")?,
                visibility_timeout: seconds_in(
                    timeout,
                    "VisibilityTimeout",
                    VISIBILITY_TIMEOUT_RANGE,
                )?,
            })
        }
        _ => Err(Error::UnknownCommand(command)),
    }
}

/// The members of a request object other than `Command`.
struct Params(Map<String, Value>);

impl Params {
    /// The value of a parameter, treating `null` as absent.
    fn optional(&mut self, name: &str) -> Option<Value> {
        self.0.remove(name).filter(|value| !value.is_null())
    }

    fn required(&mut self, name: &'static str) -> Result<Value, Error> {
        self.optional(name).ok_or(Error::MissingParameter(name))
    }

    /// The `QueueName` of a command whose only parameter it is.
    fn queue_name(&mut self) -> Result<String, Error> {
        let queue = self.required("QueueName")?;
        string(queue, "QueueName")
    }
}

fn wrong_type(parameter: &'static str, expected: &'static str) -> Error {
    Error::InvalidParameterValue {
        parameter,
        rule: expected,
    }
}

fn string(value: Value, parameter: &'static str) -> Result<String, Error> {
    match value {
        Value::String(text) => Ok(text),
        _ => Err(wrong_type(parameter, "must be a string")),
    }
}

fn boolean(value: &Value, parameter: &'static str) -> Result<bool, Error> {
    value
        .as_bool()
        .ok_or_else(|| wrong_type(parameter, "must be a boolean"))
}

/// An integer within `range`. Only numbers written without fraction or
/// exponent are integers; `i64` parsing of the source text rejects the rest.
fn integer_in(
    value: Value,
    parameter: &'static str,
    range: RangeInclusive<u32>,
) -> Result<u32, Error> {
    let Value::Number(number) = value else {
        return Err(wrong_type(parameter, "must be an integer"));
    };
    let Some(integer) = number.as_i64() else {
        return Err(wrong_type(parameter, "must be an integer"));
    };
    u32::try_from(integer)
        .ok()
        .filter(|n| range.contains(n))
        .ok_or(Error::InvalidParameterValue {
            parameter,
            rule: "is out of range",
        })
}

fn seconds_in(
    value: Value,
    parameter: &'static str,
    range: RangeInclusive<u32>,
) -> Result<Seconds, Error> {
    integer_in(value, parameter, range).map(Seconds::new)
}

fn checked<R, T>(
    raw: R,
    parameter: &'static str,
    parse: impl FnOnce(R) -> Result<T, InvalidValue>,
) -> Result<T, Error> {
    parse(raw).map_err(|InvalidValue { rule }| Error::InvalidParameterValue { parameter, rule })
}

fn queue_name(value: Value) -> Result<QueueName, Error> {
    QueueName::parse(string(value, "QueueName")?)
        .map_err(|InvalidValue { rule }| Error::QueueNameInvalid { rule })
}

fn attributes(value: Value) -> Result<AttributeChanges, Error> {
    let Value::Object(members) = value else {
        return Err(wrong_type("Attributes", "must be an object"));
    };
    let mut changes = AttributeChanges::default();
    for (name, value) in members {
        if value.is_null() {
            continue;
        }
        match name.as_str() {
            "VisibilityTimeout" => {
                changes.visibility_timeout = Some(seconds_in(
                    value,
                    "VisibilityTimeout",
                    VISIBILITY_TIMEOUT_RANGE,
                )?);
            }
            "DelaySeconds" => changes.delay = Some(seconds_in(value, "DelaySeconds", DELAY_RANGE)?),
            "RetentionSeconds" => {
                changes.retention = Some(seconds_in(value, "RetentionSeconds", RETENTION_RANGE)?);
            }
            "Fifo" => changes.fifo = Some(boolean(&value, "Fifo")?),
            "ContentBasedDeduplication" => {
                changes.content_based_deduplication =
                    Some(boolean(&value, "ContentBasedDeduplication")?);
            }
            "DeadLetterQueue" => {
                changes.dead_letter_queue = Some(string(value, "DeadLetterQueue")?);
            }
            "MaxReceiveCount" => {
                let count = integer_in(value, "MaxReceiveCount", 0..=u32::MAX)?;
                changes.max_receive_count =
                    Some(checked(count, "MaxReceiveCount", MaxReceiveCount::new)?);
            }
            _ => return Err(Error::UnknownAttribute(name)),
        }
    }
    Ok(changes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorCode;

    fn name(text: &str) -> QueueName {
        QueueName::parse(text.to_owned()).expect("test queue names are valid")
    }

    #[test]
    fn parses_well_formed_commands() {
        let cases = [
            (
                r#"{"Command":"CreateQueue","QueueName":"q.fifo","Attributes":{"Fifo":true,"DelaySeconds":null,"MaxReceiveCount":3,"DeadLetterQueue":"d.fifo"},"Extra":1}"#,
                Request::CreateQueue {
                    queue: name("q.fifo"),
                    attributes: AttributeChanges {
                        fifo: Some(true),
                        dead_letter_queue: Some("d.fifo".to_owned()),
                        max_receive_count: MaxReceiveCount::new(3).ok(),
                        ..AttributeChanges::default()
                    },
                },
            ),
            (
                r#"{"Command":"CreateQueue","QueueName":"q","Attributes":null}"#,
                Request::CreateQueue {
                    queue: name("q"),
                    attributes: AttributeChanges::default(),
                },
            ),
            (
                r#"{"Command":"Tick","Seconds":-0}"#,
                Request::Tick {
                    seconds: Seconds::new(0),
                },
            ),
            (
                r#"{"Command":"ListQueues","Prefix":null}"#,
                Request::ListQueues { prefix: None },
            ),
            (
                r#"{"Command":"ReceiveMessage","QueueName":"any thing"}"#,
                Request::ReceiveMessage(ReceiveMessage {
                    queue: "any thing".to_owned(),
                    max_messages: 1,
                    visibility_timeout: None,
                }),
            ),
            (
                " {\"Command\":\"DeleteMessage\",\"QueueName\":\"q\",\"ReceiptHandle\":\"x\"}\r",
                Request::DeleteMessage {
                    queue: "q".to_owned(),
                    receipt_handle: "x".to_owned(),
                },
            ),
        ];
        for (line, expected) in cases {
            assert_eq!(parse(line.as_bytes()), Ok(expected), "{line}");
        }
    }

    #[test]
    fn rejects_requests_with_the_first_failing_check() {
        use ErrorCode::*;
        let cases: &[(&[u8], ErrorCode)] = &[
            (b"", InvalidRequest),
            (b"[]", InvalidRequest),
            (b"{\"Command\":\"Tick\",\"Seconds\":1} x", InvalidRequest),
            (b"{\"Command\":\"Tick\",\"Seconds\":\"\xff\"}", InvalidRequest),
            (br#"{"Command":5}"#, InvalidRequest),
            (br#"{"Command":"tick","Seconds":1}"#, InvalidRequest),
            (br#"{"Command":"Tick","Seconds":null}"#, MissingParameter),
            (br#"{"Command":"Tick","Seconds":1.0}"#, InvalidParameterValue),
            (br#"{"Command":"Tick","Seconds":1e0}"#, InvalidParameterValue),
            (br#"{"Command":"Tick","Seconds":"1"}"#, InvalidParameterValue),
            (br#"{"Command":"Tick","Seconds":1209601}"#, InvalidParameterValue),
            (br#"{"Command":"Tick","Seconds":-1}"#, InvalidParameterValue),
            (br#"{"Command":"Tick","Seconds":100000000000000000000}"#, InvalidParameterValue),
            // Presence of every required parameter is checked before any value.
            (br#"{"Command":"SendMessage","QueueName":7}"#, MissingParameter),
            (br#"{"Command":"SendMessage","QueueName":"q","Body":""}"#, InvalidParameterValue),
            (br#"{"Command":"SendMessage","QueueName":"q","Body":"b","GroupId":"a b"}"#, InvalidParameterValue),
            (br#"{"Command":"ReceiveMessage","QueueName":"q","MaxMessages":11}"#, InvalidParameterValue),
            (br#"{"Command":"ReceiveMessage","QueueName":"q","MaxMessages":0}"#, InvalidParameterValue),
            (br#"{"Command":"SetQueueAttributes","QueueName":"q"}"#, MissingParameter),
            (br#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":[]}"#, InvalidParameterValue),
            (br#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"Color":1}}"#, InvalidParameterValue),
            (br#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"Fifo":"yes"}}"#, InvalidParameterValue),
            (br#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"RetentionSeconds":59}}"#, InvalidParameterValue),
            (br#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"MaxReceiveCount":0}}"#, InvalidParameterValue),
            // In CreateQueue the name is checked before every other parameter.
            (br#"{"Command":"CreateQueue","QueueName":"bad name","Attributes":5}"#, QueueNameInvalid),
            (br#"{"Command":"CreateQueue","QueueName":5,"Attributes":{}}"#, InvalidParameterValue),
            (br#"{"Command":"CreateQueue","QueueName":"q","Attributes":5}"#, InvalidParameterValue),
        ];
        for (line, code) in cases {
            assert_eq!(
                parse(line).map_err(|e| e.code()),
                Err(*code),
                "{}",
                String::from_utf8_lossy(line)
            );
        }
    }

    #[test]
    fn dead_letter_needs_both_attributes() {
        let only_queue = AttributeChanges {
            dead_letter_queue: Some("d".to_owned()),
            ..AttributeChanges::default()
        };
        assert_eq!(
            only_queue.dead_letter().map_err(|e| e.code()),
            Err(ErrorCode::InvalidParameterValue)
        );
        assert_eq!(AttributeChanges::default().dead_letter(), Ok(None));
    }
}
