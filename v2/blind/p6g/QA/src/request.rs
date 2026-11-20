//! Turns one input line into a validated `Command`.
//!
//! Checks run in the order the spec fixes: request shape, required
//! parameters, then parameter values.

use std::ops::RangeInclusive;

use serde_json::{Map, Value};

use crate::error::{Error, ErrorCode};
use crate::queue::QueueName;

/// The queue attributes of a request. A field is `None` when the request
/// leaves the attribute out or gives `null`.
#[derive(Debug, Default)]
pub(crate) struct Attributes {
    pub visibility_timeout: Option<u32>,
    pub delay_seconds: Option<u32>,
    pub retention_seconds: Option<u32>,
    pub fifo: Option<bool>,
    pub content_based_deduplication: Option<bool>,
    pub dead_letter_queue: Option<String>,
    pub max_receive_count: Option<u32>,
}

#[derive(Debug)]
pub(crate) struct Send {
    pub queue: String,
    pub body: String,
    pub delay_seconds: Option<u32>,
    pub group_id: Option<String>,
    pub deduplication_id: Option<String>,
}

#[derive(Debug)]
pub(crate) enum Command {
    CreateQueue {
        name: QueueName,
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
    SendMessage(Send),
    ReceiveMessage {
        queue: String,
        max_messages: u32,
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

fn invalid_value(name: &str, expected: &str) -> Error {
    Error::new(
        ErrorCode::InvalidParameterValue,
        format!("{name} must be {expected}"),
    )
}

/// Typed access to the members of a JSON object, where `null` is absent.
#[derive(Clone, Copy)]
struct Params<'a>(&'a Map<String, Value>);

impl<'a> Params<'a> {
    fn get(self, name: &str) -> Option<&'a Value> {
        self.0.get(name).filter(|value| !value.is_null())
    }

    fn require(self, names: &[&str]) -> Result<(), Error> {
        match names.iter().find(|name| self.get(name).is_none()) {
            Some(name) => Err(Error::new(
                ErrorCode::MissingParameter,
                format!("{name} is required"),
            )),
            None => Ok(()),
        }
    }

    fn string(self, name: &str) -> Result<Option<&'a str>, Error> {
        match self.get(name) {
            None => Ok(None),
            Some(Value::String(text)) => Ok(Some(text)),
            Some(_) => Err(invalid_value(name, "a string")),
        }
    }

    fn required_string(self, name: &str) -> Result<&'a str, Error> {
        self.string(name)?
            .ok_or_else(|| Error::new(ErrorCode::MissingParameter, format!("{name} is required")))
    }

    fn boolean(self, name: &str) -> Result<Option<bool>, Error> {
        match self.get(name) {
            None => Ok(None),
            Some(Value::Bool(flag)) => Ok(Some(*flag)),
            Some(_) => Err(invalid_value(name, "a boolean")),
        }
    }

    fn integer(self, name: &str, range: RangeInclusive<u32>) -> Result<Option<u32>, Error> {
        let Some(value) = self.get(name) else {
            return Ok(None);
        };
        // `as_i64` is `None` for numbers written with a fraction or exponent.
        value
            .as_i64()
            .and_then(|n| u32::try_from(n).ok())
            .filter(|n| range.contains(n))
            .map(Some)
            .ok_or_else(|| {
                invalid_value(
                    name,
                    &format!("an integer from {} to {}", range.start(), range.end()),
                )
            })
    }

    fn required_integer(self, name: &str, range: RangeInclusive<u32>) -> Result<u32, Error> {
        self.integer(name, range)?
            .ok_or_else(|| Error::new(ErrorCode::MissingParameter, format!("{name} is required")))
    }

    fn object(self, name: &str) -> Result<Option<&'a Map<String, Value>>, Error> {
        match self.get(name) {
            None => Ok(None),
            Some(Value::Object(map)) => Ok(Some(map)),
            Some(_) => Err(invalid_value(name, "an object")),
        }
    }

    fn required_object(self, name: &str) -> Result<&'a Map<String, Value>, Error> {
        self.object(name)?
            .ok_or_else(|| Error::new(ErrorCode::MissingParameter, format!("{name} is required")))
    }

    fn identifier(self, name: &str) -> Result<Option<&'a str>, Error> {
        let Some(text) = self.string(name)? else {
            return Ok(None);
        };
        let valid = (1..=128).contains(&text.chars().count())
            && text
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
        if valid {
            Ok(Some(text))
        } else {
            Err(invalid_value(
                name,
                "1 to 128 characters from A-Z a-z 0-9 _ -",
            ))
        }
    }
}

impl Attributes {
    fn parse(map: &Map<String, Value>) -> Result<Self, Error> {
        const KNOWN: [&str; 7] = [
            "VisibilityTimeout",
            "DelaySeconds",
            "RetentionSeconds",
            "Fifo",
            "ContentBasedDeduplication",
            "DeadLetterQueue",
            "MaxReceiveCount",
        ];
        let p = Params(map);
        if let Some(name) = map
            .keys()
            .find(|name| !KNOWN.contains(&name.as_str()) && p.get(name).is_some())
        {
            return Err(invalid_value("Attributes", &format!("without {name}")));
        }
        Ok(Self {
            visibility_timeout: p.integer("VisibilityTimeout", 0..=43_200)?,
            delay_seconds: p.integer("DelaySeconds", 0..=900)?,
            retention_seconds: p.integer("RetentionSeconds", 60..=1_209_600)?,
            fifo: p.boolean("Fifo")?,
            content_based_deduplication: p.boolean("ContentBasedDeduplication")?,
            dead_letter_queue: p.string("DeadLetterQueue")?.map(str::to_owned),
            max_receive_count: p.integer("MaxReceiveCount", 1..=1000)?,
        })
    }
}

impl Command {
    pub(crate) fn parse(line: &[u8]) -> Result<Self, Error> {
        let bad_request = |message: &str| Error::new(ErrorCode::InvalidRequest, message);
        let text = std::str::from_utf8(line).map_err(|_| bad_request("line is not valid UTF-8"))?;
        let Value::Object(map) =
            serde_json::from_str(text).map_err(|e| bad_request(&format!("not JSON: {e}")))?
        else {
            return Err(bad_request("line is not a JSON object"));
        };
        let Some(Value::String(command)) = map.get("Command") else {
            return Err(bad_request("Command must be a string"));
        };
        let p = Params(&map);
        match command.as_str() {
            "CreateQueue" => {
                p.require(&["QueueName"])?;
                let name = p.required_string("QueueName")?;
                let name = QueueName::parse(name).ok_or_else(|| {
                    Error::new(
                        ErrorCode::QueueNameInvalid,
                        format!("invalid queue name {name:?}"),
                    )
                })?;
                let attributes = p.object("Attributes")?.map(Attributes::parse).transpose()?;
                Ok(Self::CreateQueue {
                    name,
                    attributes: attributes.unwrap_or_default(),
                })
            }
            "SetQueueAttributes" => {
                p.require(&["QueueName", "Attributes"])?;
                let queue = p.required_string("QueueName")?.to_owned();
                let attributes = Attributes::parse(p.required_object("Attributes")?)?;
                Ok(Self::SetQueueAttributes { queue, attributes })
            }
            "GetQueueAttributes" => Ok(Self::GetQueueAttributes {
                queue: queue_name(p)?,
            }),
            "ListQueues" => Ok(Self::ListQueues {
                prefix: p.string("Prefix")?.map(str::to_owned),
            }),
            "DeleteQueue" => Ok(Self::DeleteQueue {
                queue: queue_name(p)?,
            }),
            "PurgeQueue" => Ok(Self::PurgeQueue {
                queue: queue_name(p)?,
            }),
            "Tick" => {
                p.require(&["Seconds"])?;
                Ok(Self::Tick {
                    seconds: p.required_integer("Seconds", 0..=1_209_600)?,
                })
            }
            "SendMessage" => {
                p.require(&["QueueName", "Body"])?;
                let queue = p.required_string("QueueName")?.to_owned();
                let body = p.required_string("Body")?;
                if !(1..=1024).contains(&body.len()) {
                    return Err(invalid_value("Body", "1 to 1024 bytes"));
                }
                Ok(Self::SendMessage(Send {
                    queue,
                    body: body.to_owned(),
                    delay_seconds: p.integer("DelaySeconds", 0..=900)?,
                    group_id: p.identifier("GroupId")?.map(str::to_owned),
                    deduplication_id: p.identifier("DeduplicationId")?.map(str::to_owned),
                }))
            }
            "ReceiveMessage" => {
                p.require(&["QueueName"])?;
                let queue = p.required_string("QueueName")?.to_owned();
                Ok(Self::ReceiveMessage {
                    queue,
                    max_messages: p.integer("MaxMessages", 1..=10)?.unwrap_or(1),
                    visibility_timeout: p.integer("VisibilityTimeout", 0..=43_200)?,
                })
            }
            "DeleteMessage" => {
                p.require(&["QueueName", "ReceiptHandle"])?;
                Ok(Self::DeleteMessage {
                    queue: p.required_string("QueueName")?.to_owned(),
                    receipt_handle: p.required_string("ReceiptHandle")?.to_owned(),
                })
            }
            "ChangeMessageVisibility" => {
                p.require(&["QueueName", "ReceiptHandle", "VisibilityTimeout"])?;
                Ok(Self::ChangeMessageVisibility {
                    queue: p.required_string("QueueName")?.to_owned(),
                    receipt_handle: p.required_string("ReceiptHandle")?.to_owned(),
                    visibility_timeout: p.required_integer("VisibilityTimeout", 0..=43_200)?,
                })
            }
            other => Err(bad_request(&format!("unknown command {other:?}"))),
        }
    }
}

fn queue_name(p: Params<'_>) -> Result<String, Error> {
    p.require(&["QueueName"])?;
    Ok(p.required_string("QueueName")?.to_owned())
}
