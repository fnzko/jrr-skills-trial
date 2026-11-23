//! Turns one input line into a validated request.
//!
//! Checks run in the order of the specification: request shape, required
//! parameters, then parameter values.

use std::ops::RangeInclusive;

use serde_json::{Map, Value};

use crate::error::{Error, ErrorCode};
use crate::types::{Body, DeduplicationId, GroupId, QueueName, Seconds, TICK_RANGE};

const VISIBILITY_TIMEOUT_RANGE: RangeInclusive<u32> = 0..=43_200;
const DELAY_RANGE: RangeInclusive<u32> = 0..=900;
const RETENTION_RANGE: RangeInclusive<u32> = 60..=1_209_600;
const MAX_MESSAGES_RANGE: RangeInclusive<u32> = 1..=10;
const MAX_RECEIVE_COUNT_RANGE: RangeInclusive<u32> = 1..=1000;

/// A command name the protocol knows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Command {
    CreateQueue,
    SetQueueAttributes,
    GetQueueAttributes,
    ListQueues,
    DeleteQueue,
    PurgeQueue,
    Tick,
    SendMessage,
    ReceiveMessage,
    DeleteMessage,
    ChangeMessageVisibility,
}

impl Command {
    fn from_name(name: &str) -> Option<Self> {
        match name {
            "CreateQueue" => Some(Self::CreateQueue),
            "SetQueueAttributes" => Some(Self::SetQueueAttributes),
            "GetQueueAttributes" => Some(Self::GetQueueAttributes),
            "ListQueues" => Some(Self::ListQueues),
            "DeleteQueue" => Some(Self::DeleteQueue),
            "PurgeQueue" => Some(Self::PurgeQueue),
            "Tick" => Some(Self::Tick),
            "SendMessage" => Some(Self::SendMessage),
            "ReceiveMessage" => Some(Self::ReceiveMessage),
            "DeleteMessage" => Some(Self::DeleteMessage),
            "ChangeMessageVisibility" => Some(Self::ChangeMessageVisibility),
            _ => None,
        }
    }
}

/// The queue attributes a request names; each is absent unless the request gave it.
///
/// The dead-letter pair is kept apart because "exactly one of the two" is a rule
/// of the commands, which run after other checks.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct AttributeSet<'a> {
    pub visibility_timeout: Option<Seconds>,
    pub delay: Option<Seconds>,
    pub retention: Option<Seconds>,
    pub fifo: Option<bool>,
    pub content_based_deduplication: Option<bool>,
    pub dead_letter_queue: Option<&'a str>,
    pub max_receive_count: Option<u32>,
}

impl<'a> AttributeSet<'a> {
    fn parse(members: &'a Map<String, Value>) -> Result<Self, Error> {
        const KNOWN: [&str; 7] = [
            "VisibilityTimeout",
            "DelaySeconds",
            "RetentionSeconds",
            "Fifo",
            "ContentBasedDeduplication",
            "DeadLetterQueue",
            "MaxReceiveCount",
        ];
        let params = Params(members);
        if let Some(name) = params.present_names().find(|name| !KNOWN.contains(name)) {
            return Err(Error::invalid(
                "Attributes",
                &format!("has unknown attribute {name}"),
            ));
        }
        Ok(Self {
            visibility_timeout: params
                .optional_integer("VisibilityTimeout", VISIBILITY_TIMEOUT_RANGE)?
                .map(Seconds::new),
            delay: params
                .optional_integer("DelaySeconds", DELAY_RANGE)?
                .map(Seconds::new),
            retention: params
                .optional_integer("RetentionSeconds", RETENTION_RANGE)?
                .map(Seconds::new),
            fifo: params.optional_boolean("Fifo")?,
            content_based_deduplication: params.optional_boolean("ContentBasedDeduplication")?,
            dead_letter_queue: params.optional_string("DeadLetterQueue")?,
            max_receive_count: params
                .optional_integer("MaxReceiveCount", MAX_RECEIVE_COUNT_RANGE)?,
        })
    }

    /// The dead-letter configuration the request asks for: both members or neither.
    pub fn dead_letter(&self) -> Result<Option<(&'a str, u32)>, Error> {
        match (self.dead_letter_queue, self.max_receive_count) {
            (Some(queue), Some(count)) => Ok(Some((queue, count))),
            (None, None) => Ok(None),
            _ => Err(Error::invalid(
                "Attributes",
                "must give DeadLetterQueue and MaxReceiveCount together",
            )),
        }
    }
}

/// A validated `SendMessage`.
#[derive(Debug, PartialEq, Eq)]
pub struct SendRequest<'a> {
    pub queue: &'a str,
    pub body: Body,
    pub delay: Option<Seconds>,
    pub group: Option<GroupId>,
    pub deduplication_id: Option<DeduplicationId>,
}

/// A validated command with its parameters.
#[derive(Debug, PartialEq, Eq)]
pub enum Request<'a> {
    CreateQueue {
        name: QueueName,
        attributes: AttributeSet<'a>,
    },
    SetQueueAttributes {
        queue: &'a str,
        attributes: AttributeSet<'a>,
    },
    GetQueueAttributes {
        queue: &'a str,
    },
    ListQueues {
        prefix: Option<&'a str>,
    },
    DeleteQueue {
        queue: &'a str,
    },
    PurgeQueue {
        queue: &'a str,
    },
    Tick {
        seconds: Seconds,
    },
    SendMessage(SendRequest<'a>),
    ReceiveMessage {
        queue: &'a str,
        max_messages: u32,
        visibility_timeout: Option<Seconds>,
    },
    DeleteMessage {
        queue: &'a str,
        receipt_handle: &'a str,
    },
    ChangeMessageVisibility {
        queue: &'a str,
        receipt_handle: &'a str,
        visibility_timeout: Seconds,
    },
}

/// A line that is a JSON object with a known `Command`, parameters not yet checked.
pub struct Document {
    command: Command,
    members: Map<String, Value>,
}

impl Document {
    pub fn parse(line: &[u8]) -> Result<Self, Error> {
        let value: Value = serde_json::from_slice(line).map_err(|e| {
            Error::new(
                ErrorCode::InvalidRequest,
                format!("line is not valid JSON: {e}"),
            )
        })?;
        let Value::Object(members) = value else {
            return Err(Error::new(
                ErrorCode::InvalidRequest,
                "line is not a JSON object",
            ));
        };
        let Some(Value::String(name)) = members.get("Command") else {
            return Err(Error::new(
                ErrorCode::InvalidRequest,
                "member Command is missing or not a string",
            ));
        };
        let command = Command::from_name(name).ok_or_else(|| {
            Error::new(ErrorCode::InvalidRequest, format!("unknown command {name}"))
        })?;
        Ok(Self { command, members })
    }

    pub fn request(&self) -> Result<Request<'_>, Error> {
        let params = Params(&self.members);
        match self.command {
            Command::CreateQueue => {
                params.require(&["QueueName"])?;
                // The name is judged before every other parameter.
                let name = QueueName::parse(params.required_string("QueueName")?)?;
                let attributes = params.optional_attributes()?.unwrap_or_default();
                Ok(Request::CreateQueue { name, attributes })
            }
            Command::SetQueueAttributes => {
                params.require(&["QueueName", "Attributes"])?;
                let queue = params.required_string("QueueName")?;
                let attributes = params
                    .optional_attributes()?
                    .ok_or_else(|| Error::missing("Attributes"))?;
                Ok(Request::SetQueueAttributes { queue, attributes })
            }
            Command::GetQueueAttributes => Ok(Request::GetQueueAttributes {
                queue: params.queue()?,
            }),
            Command::ListQueues => Ok(Request::ListQueues {
                prefix: params.optional_string("Prefix")?,
            }),
            Command::DeleteQueue => Ok(Request::DeleteQueue {
                queue: params.queue()?,
            }),
            Command::PurgeQueue => Ok(Request::PurgeQueue {
                queue: params.queue()?,
            }),
            Command::Tick => {
                params.require(&["Seconds"])?;
                let seconds = params.required_integer("Seconds", TICK_RANGE)?;
                Ok(Request::Tick {
                    seconds: Seconds::new(seconds),
                })
            }
            Command::ReceiveMessage => {
                params.require(&["QueueName"])?;
                Ok(Request::ReceiveMessage {
                    queue: params.required_string("QueueName")?,
                    max_messages: params
                        .optional_integer("MaxMessages", MAX_MESSAGES_RANGE)?
                        .unwrap_or(1),
                    visibility_timeout: params
                        .optional_integer("VisibilityTimeout", VISIBILITY_TIMEOUT_RANGE)?
                        .map(Seconds::new),
                })
            }
            Command::DeleteMessage => {
                params.require(&["QueueName", "ReceiptHandle"])?;
                Ok(Request::DeleteMessage {
                    queue: params.required_string("QueueName")?,
                    receipt_handle: params.required_string("ReceiptHandle")?,
                })
            }
            Command::ChangeMessageVisibility => {
                params.require(&["QueueName", "ReceiptHandle", "VisibilityTimeout"])?;
                Ok(Request::ChangeMessageVisibility {
                    queue: params.required_string("QueueName")?,
                    receipt_handle: params.required_string("ReceiptHandle")?,
                    visibility_timeout: Seconds::new(
                        params.required_integer("VisibilityTimeout", VISIBILITY_TIMEOUT_RANGE)?,
                    ),
                })
            }
            Command::SendMessage => {
                params.require(&["QueueName", "Body"])?;
                Ok(Request::SendMessage(SendRequest {
                    queue: params.required_string("QueueName")?,
                    body: Body::parse(params.required_string("Body")?)?,
                    delay: params
                        .optional_integer("DelaySeconds", DELAY_RANGE)?
                        .map(Seconds::new),
                    group: params
                        .optional_string("GroupId")?
                        .map(GroupId::parse)
                        .transpose()?,
                    deduplication_id: params
                        .optional_string("DeduplicationId")?
                        .map(DeduplicationId::parse)
                        .transpose()?,
                }))
            }
        }
    }
}

/// Read access to the parameters of one command; `null` counts as absent.
#[derive(Clone, Copy)]
struct Params<'a>(&'a Map<String, Value>);

impl<'a> Params<'a> {
    fn get(self, name: &str) -> Option<&'a Value> {
        self.0.get(name).filter(|value| !value.is_null())
    }

    fn present_names(self) -> impl Iterator<Item = &'a str> {
        self.0
            .iter()
            .filter(|(_, value)| !value.is_null())
            .map(|(name, _)| name.as_str())
    }

    /// Fails with the first of `names` that is absent.
    fn require(self, names: &[&str]) -> Result<(), Error> {
        match names.iter().find(|name| self.get(name).is_none()) {
            Some(name) => Err(Error::missing(name)),
            None => Ok(()),
        }
    }

    fn optional_string(self, name: &str) -> Result<Option<&'a str>, Error> {
        self.get(name)
            .map(|value| {
                value
                    .as_str()
                    .ok_or_else(|| Error::invalid(name, "must be a string"))
            })
            .transpose()
    }

    fn required_string(self, name: &str) -> Result<&'a str, Error> {
        self.optional_string(name)?
            .ok_or_else(|| Error::missing(name))
    }

    /// The `QueueName` of the commands that take nothing else but may name any string.
    fn queue(self) -> Result<&'a str, Error> {
        self.require(&["QueueName"])?;
        self.required_string("QueueName")
    }

    fn optional_boolean(self, name: &str) -> Result<Option<bool>, Error> {
        self.get(name)
            .map(|value| {
                value
                    .as_bool()
                    .ok_or_else(|| Error::invalid(name, "must be a boolean"))
            })
            .transpose()
    }

    fn optional_integer(
        self,
        name: &str,
        range: RangeInclusive<u32>,
    ) -> Result<Option<u32>, Error> {
        self.get(name)
            .map(|value| {
                // `as_i64` rejects fractions and exponents because numbers keep their source text.
                let number = value
                    .as_number()
                    .and_then(|n| n.as_i64())
                    .ok_or_else(|| Error::invalid(name, "must be an integer"))?;
                u32::try_from(number)
                    .ok()
                    .filter(|n| range.contains(n))
                    .ok_or_else(|| {
                        Error::invalid(
                            name,
                            &format!("must be between {} and {}", range.start(), range.end()),
                        )
                    })
            })
            .transpose()
    }

    fn required_integer(self, name: &str, range: RangeInclusive<u32>) -> Result<u32, Error> {
        self.optional_integer(name, range)?
            .ok_or_else(|| Error::missing(name))
    }

    fn optional_attributes(self) -> Result<Option<AttributeSet<'a>>, Error> {
        self.get("Attributes")
            .map(|value| match value {
                Value::Object(members) => AttributeSet::parse(members),
                Value::Null
                | Value::Bool(_)
                | Value::Number(_)
                | Value::String(_)
                | Value::Array(_) => Err(Error::invalid("Attributes", "must be an object")),
            })
            .transpose()
    }
}
