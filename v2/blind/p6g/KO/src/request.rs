//! Turns one input line into a typed [`Request`].
//!
//! Parsing follows the order of the specification: request shape, then
//! required parameters, then the value of each parameter.

use std::ops::RangeInclusive;

use serde_json::{Map, Value};

use crate::error::{Error, ErrorCode};
use crate::queue::DeadLetter;

#[derive(Debug, PartialEq, Eq)]
pub enum Request {
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
    SendMessage(SendMessage),
    ReceiveMessage {
        queue: String,
        max_messages: u32,
        /// Overrides the queue's attribute for this call.
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
    Tick {
        seconds: u32,
    },
}

#[derive(Debug, PartialEq, Eq)]
pub struct SendMessage {
    pub queue: String,
    pub body: String,
    pub delay_seconds: Option<u32>,
    pub group_id: Option<String>,
    pub deduplication_id: Option<String>,
}

/// The attributes present in a request; every one of them is optional.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Attributes {
    pub visibility_timeout: Option<u32>,
    pub delay_seconds: Option<u32>,
    pub retention_seconds: Option<u32>,
    pub fifo: Option<bool>,
    pub content_based_deduplication: Option<bool>,
    dead_letter_queue: Option<String>,
    max_receive_count: Option<u32>,
}

impl Attributes {
    /// The dead-letter configuration of the request: both attributes or neither.
    pub fn dead_letter(&mut self) -> Result<Option<DeadLetter>, Error> {
        match (self.dead_letter_queue.take(), self.max_receive_count.take()) {
            (Some(queue), Some(max_receive_count)) => Ok(Some(DeadLetter {
                queue,
                max_receive_count,
            })),
            (None, None) => Ok(None),
            _ => Err(Error::invalid_value(
                "DeadLetterQueue and MaxReceiveCount must be given together",
            )),
        }
    }

    fn parse(value: Value) -> Result<Self, Error> {
        let Value::Object(members) = value else {
            return Err(Error::invalid_value("Attributes must be an object"));
        };
        let mut attributes = Self::default();
        for (name, value) in members.into_iter().filter(|(_, v)| !v.is_null()) {
            match name.as_str() {
                "VisibilityTimeout" => {
                    attributes.visibility_timeout = Some(integer(&name, &value, 0..=43_200)?);
                }
                "DelaySeconds" => {
                    attributes.delay_seconds = Some(integer(&name, &value, 0..=900)?);
                }
                "RetentionSeconds" => {
                    attributes.retention_seconds = Some(integer(&name, &value, 60..=1_209_600)?);
                }
                "Fifo" => attributes.fifo = Some(boolean(&name, &value)?),
                "ContentBasedDeduplication" => {
                    attributes.content_based_deduplication = Some(boolean(&name, &value)?);
                }
                "DeadLetterQueue" => attributes.dead_letter_queue = Some(string(&name, value)?),
                "MaxReceiveCount" => {
                    attributes.max_receive_count = Some(integer(&name, &value, 1..=1000)?);
                }
                _ => {
                    return Err(Error::invalid_value(format!("unknown attribute {name:?}")));
                }
            }
        }
        Ok(attributes)
    }
}

/// A queue name is 1 to 80 characters of `A-Z a-z 0-9 _ -`, or 1 to 75 of
/// them followed by `.fifo`.
fn is_valid_queue_name(name: &str) -> bool {
    let stem = name.strip_suffix(".fifo").unwrap_or(name);
    let max_len = if stem.len() == name.len() { 80 } else { 75 };
    (1..=max_len).contains(&stem.len())
        && stem
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// The members of a command object, with `null` members already dropped.
struct Params(Map<String, Value>);

impl Params {
    fn take(&mut self, name: &str) -> Option<Value> {
        self.0.remove(name).filter(|value| !value.is_null())
    }

    /// The required `QueueName` of commands that have no other parameter.
    fn queue_name(&mut self) -> Result<String, Error> {
        string("QueueName", self.require("QueueName")?)
    }

    fn require(&mut self, name: &str) -> Result<Value, Error> {
        self.take(name).ok_or_else(|| Error::missing(name))
    }
}

fn string(name: &str, value: Value) -> Result<String, Error> {
    match value {
        Value::String(s) => Ok(s),
        _ => Err(Error::invalid_value(format!("{name} must be a string"))),
    }
}

/// A group or deduplication id: 1 to 128 characters of `A-Z a-z 0-9 _ -`.
fn identifier(name: &str, value: Value) -> Result<String, Error> {
    let id = string(name, value)?;
    let valid = (1..=128).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
    if valid {
        Ok(id)
    } else {
        Err(Error::invalid_value(format!(
            "{name} must be 1 to 128 characters of A-Z a-z 0-9 _ -"
        )))
    }
}

fn boolean(name: &str, value: &Value) -> Result<bool, Error> {
    value
        .as_bool()
        .ok_or_else(|| Error::invalid_value(format!("{name} must be a boolean")))
}

fn integer(name: &str, value: &Value, range: RangeInclusive<u32>) -> Result<u32, Error> {
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

impl Request {
    pub fn parse(line: &[u8]) -> Result<Self, Error> {
        let text = std::str::from_utf8(line)
            .map_err(|e| Error::invalid_request(format!("line is not valid UTF-8: {e}")))?;
        let value: Value = serde_json::from_str(text)
            .map_err(|e| Error::invalid_request(format!("line is not valid JSON: {e}")))?;
        let Value::Object(members) = value else {
            return Err(Error::invalid_request("line is not a JSON object"));
        };
        let mut params = Params(members);
        let Some(Value::String(command)) = params.take("Command") else {
            return Err(Error::invalid_request("Command must be a string"));
        };

        match command.as_str() {
            "CreateQueue" => {
                let name = params.require("QueueName")?;
                let attributes = params.take("Attributes");
                // The name is checked before every other parameter.
                let name = string("QueueName", name)?;
                if !is_valid_queue_name(&name) {
                    return Err(Error::new(
                        ErrorCode::QueueNameInvalid,
                        format!("{name:?} is not a valid queue name"),
                    ));
                }
                let attributes = attributes.map(Attributes::parse).transpose()?;
                Ok(Self::CreateQueue {
                    name,
                    attributes: attributes.unwrap_or_default(),
                })
            }
            "SetQueueAttributes" => {
                let queue = params.require("QueueName")?;
                let attributes = params.require("Attributes")?;
                Ok(Self::SetQueueAttributes {
                    queue: string("QueueName", queue)?,
                    attributes: Attributes::parse(attributes)?,
                })
            }
            "GetQueueAttributes" => Ok(Self::GetQueueAttributes {
                queue: params.queue_name()?,
            }),
            "ListQueues" => Ok(Self::ListQueues {
                prefix: params
                    .take("Prefix")
                    .map(|prefix| string("Prefix", prefix))
                    .transpose()?,
            }),
            "DeleteQueue" => Ok(Self::DeleteQueue {
                queue: params.queue_name()?,
            }),
            "PurgeQueue" => Ok(Self::PurgeQueue {
                queue: params.queue_name()?,
            }),
            "SendMessage" => {
                let queue = params.require("QueueName")?;
                let body = params.require("Body")?;
                let queue = string("QueueName", queue)?;
                let body = string("Body", body)?;
                if !(1..=1024).contains(&body.len()) {
                    return Err(Error::invalid_value("Body must be 1 to 1024 bytes"));
                }
                let delay_seconds = params
                    .take("DelaySeconds")
                    .map(|v| integer("DelaySeconds", &v, 0..=900))
                    .transpose()?;
                let group_id = params
                    .take("GroupId")
                    .map(|v| identifier("GroupId", v))
                    .transpose()?;
                let deduplication_id = params
                    .take("DeduplicationId")
                    .map(|v| identifier("DeduplicationId", v))
                    .transpose()?;
                Ok(Self::SendMessage(SendMessage {
                    queue,
                    body,
                    delay_seconds,
                    group_id,
                    deduplication_id,
                }))
            }
            "ReceiveMessage" => {
                let queue = params.queue_name()?;
                let max_messages = params
                    .take("MaxMessages")
                    .map(|v| integer("MaxMessages", &v, 1..=10))
                    .transpose()?;
                let visibility_timeout = params
                    .take("VisibilityTimeout")
                    .map(|v| integer("VisibilityTimeout", &v, 0..=43_200))
                    .transpose()?;
                Ok(Self::ReceiveMessage {
                    queue,
                    max_messages: max_messages.unwrap_or(1),
                    visibility_timeout,
                })
            }
            "DeleteMessage" => {
                let queue = params.require("QueueName")?;
                let receipt_handle = params.require("ReceiptHandle")?;
                Ok(Self::DeleteMessage {
                    queue: string("QueueName", queue)?,
                    receipt_handle: string("ReceiptHandle", receipt_handle)?,
                })
            }
            "ChangeMessageVisibility" => {
                let queue = params.require("QueueName")?;
                let receipt_handle = params.require("ReceiptHandle")?;
                let visibility_timeout = params.require("VisibilityTimeout")?;
                Ok(Self::ChangeMessageVisibility {
                    queue: string("QueueName", queue)?,
                    receipt_handle: string("ReceiptHandle", receipt_handle)?,
                    visibility_timeout: integer(
                        "VisibilityTimeout",
                        &visibility_timeout,
                        0..=43_200,
                    )?,
                })
            }
            "Tick" => {
                let seconds = params.require("Seconds")?;
                Ok(Self::Tick {
                    seconds: integer("Seconds", &seconds, 0..=1_209_600)?,
                })
            }
            other => Err(Error::invalid_request(format!("unknown command {other:?}"))),
        }
    }
}
