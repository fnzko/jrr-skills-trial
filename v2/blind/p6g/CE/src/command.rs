//! Turns one input line into a typed command, applying the checks of the
//! error-order rules: shape, then required parameters, then values.

use serde_json::{Map, Value};

use crate::error::{Error, ErrorCode};
use crate::scalar::{
    Bounded, DelaySeconds, MaxMessages, MaxReceiveCount, RetentionSeconds, TickSeconds,
    VisibilityTimeout,
};
use crate::text::{Body, DeduplicationId, GroupId, QueueName};

const ATTRIBUTE_NAMES: [&str; 7] = [
    "VisibilityTimeout",
    "DelaySeconds",
    "RetentionSeconds",
    "Fifo",
    "ContentBasedDeduplication",
    "DeadLetterQueue",
    "MaxReceiveCount",
];

/// The attributes a request gave; absent ones are `None`.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct AttributePatch {
    pub visibility_timeout: Option<VisibilityTimeout>,
    pub delay: Option<DelaySeconds>,
    pub retention: Option<RetentionSeconds>,
    pub fifo: Option<bool>,
    pub content_based_deduplication: Option<bool>,
    pub dead_letter_queue: Option<String>,
    pub max_receive_count: Option<MaxReceiveCount>,
}

impl AttributePatch {
    /// The dead-letter configuration, which is given completely or not at all.
    pub fn dead_letter(&self) -> Result<Option<(&str, MaxReceiveCount)>, Error> {
        match (&self.dead_letter_queue, self.max_receive_count) {
            (Some(queue), Some(count)) => Ok(Some((queue, count))),
            (None, None) => Ok(None),
            (Some(_), None) | (None, Some(_)) => Err(Error::new(
                ErrorCode::InvalidParameterValue,
                "DeadLetterQueue and MaxReceiveCount must be given together",
            )),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct SendRequest {
    pub queue: String,
    pub body: Body,
    pub delay: Option<DelaySeconds>,
    pub group: Option<GroupId>,
    pub dedup_id: Option<DeduplicationId>,
}

#[derive(Debug, PartialEq, Eq)]
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
        seconds: TickSeconds,
    },
    SendMessage(SendRequest),
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
}

impl Command {
    pub fn parse(line: &[u8]) -> Result<Self, Error> {
        let text = std::str::from_utf8(line)
            .map_err(|_| Error::new(ErrorCode::InvalidRequest, "the line is not valid UTF-8"))?;
        let value: Value = serde_json::from_str(text).map_err(|source| {
            Error::new(
                ErrorCode::InvalidRequest,
                format!("the line is not valid JSON: {source}"),
            )
        })?;
        let Value::Object(members) = value else {
            return Err(Error::new(
                ErrorCode::InvalidRequest,
                "the line is not a JSON object",
            ));
        };
        let params = Params(&members);
        let name = match members.get("Command") {
            Some(Value::String(name)) => name.as_str(),
            _ => {
                return Err(Error::new(
                    ErrorCode::InvalidRequest,
                    "Command is missing or not a string",
                ));
            }
        };
        match name {
            "CreateQueue" => {
                params.require(&["QueueName"])?;
                let name = params.queue_name()?;
                Ok(Self::CreateQueue {
                    name,
                    attributes: params.attributes()?.unwrap_or_default(),
                })
            }
            "SetQueueAttributes" => {
                params.require(&["QueueName", "Attributes"])?;
                Ok(Self::SetQueueAttributes {
                    queue: params.queue()?,
                    attributes: params.attributes()?.unwrap_or_default(),
                })
            }
            "GetQueueAttributes" => Ok(Self::GetQueueAttributes {
                queue: params.queue_only()?,
            }),
            "ListQueues" => Ok(Self::ListQueues {
                prefix: params.string("Prefix")?.map(str::to_owned),
            }),
            "DeleteQueue" => Ok(Self::DeleteQueue {
                queue: params.queue_only()?,
            }),
            "PurgeQueue" => Ok(Self::PurgeQueue {
                queue: params.queue_only()?,
            }),
            "Tick" => {
                params.require(&["Seconds"])?;
                Ok(Self::Tick {
                    seconds: params.required_bounded("Seconds")?,
                })
            }
            "SendMessage" => {
                params.require(&["QueueName", "Body"])?;
                let queue = params.queue()?;
                let body = params.parsed("Body", Body::parse)?;
                Ok(Self::SendMessage(SendRequest {
                    queue,
                    body: params.present(body, "Body")?,
                    delay: params.bounded("DelaySeconds")?,
                    group: params.parsed("GroupId", GroupId::parse)?,
                    dedup_id: params.parsed("DeduplicationId", DeduplicationId::parse)?,
                }))
            }
            "ReceiveMessage" => Ok(Self::ReceiveMessage {
                queue: params.queue_only()?,
                max_messages: params
                    .bounded("MaxMessages")?
                    .unwrap_or(MaxMessages::literal::<1>()),
                visibility_timeout: params.bounded("VisibilityTimeout")?,
            }),
            "DeleteMessage" => {
                params.require(&["QueueName", "ReceiptHandle"])?;
                Ok(Self::DeleteMessage {
                    queue: params.queue()?,
                    receipt_handle: params.required_string("ReceiptHandle")?,
                })
            }
            "ChangeMessageVisibility" => {
                params.require(&["QueueName", "ReceiptHandle", "VisibilityTimeout"])?;
                Ok(Self::ChangeMessageVisibility {
                    queue: params.queue()?,
                    receipt_handle: params.required_string("ReceiptHandle")?,
                    visibility_timeout: params.required_bounded("VisibilityTimeout")?,
                })
            }
            other => Err(Error::new(
                ErrorCode::InvalidRequest,
                format!("unknown command {other:?}"),
            )),
        }
    }
}

/// The members of a JSON object; a `null` member counts as absent.
struct Params<'a>(&'a Map<String, Value>);

impl Params<'_> {
    fn get(&self, name: &str) -> Option<&Value> {
        self.0.get(name).filter(|value| !value.is_null())
    }

    fn require(&self, names: &[&str]) -> Result<(), Error> {
        match names.iter().find(|name| self.get(name).is_none()) {
            Some(name) => Err(Error::new(
                ErrorCode::MissingParameter,
                format!("parameter {name} is required"),
            )),
            None => Ok(()),
        }
    }

    /// Turns a value that `require` already guaranteed into its non-optional form.
    fn present<T>(&self, value: Option<T>, name: &str) -> Result<T, Error> {
        value.ok_or_else(|| {
            Error::new(
                ErrorCode::MissingParameter,
                format!("parameter {name} is required"),
            )
        })
    }

    fn string(&self, name: &str) -> Result<Option<&str>, Error> {
        match self.get(name) {
            None => Ok(None),
            Some(Value::String(text)) => Ok(Some(text)),
            Some(_) => Err(Error::invalid_parameter(name)),
        }
    }

    fn required_string(&self, name: &str) -> Result<String, Error> {
        let text = self.string(name)?;
        self.present(text, name).map(str::to_owned)
    }

    fn parsed<T>(&self, name: &str, parse: fn(&str) -> Option<T>) -> Result<Option<T>, Error> {
        self.string(name)?
            .map(|text| parse(text).ok_or_else(|| Error::invalid_parameter(name)))
            .transpose()
    }

    fn boolean(&self, name: &str) -> Result<Option<bool>, Error> {
        match self.get(name) {
            None => Ok(None),
            Some(Value::Bool(flag)) => Ok(Some(*flag)),
            Some(_) => Err(Error::invalid_parameter(name)),
        }
    }

    fn bounded<const MIN: u32, const MAX: u32>(
        &self,
        name: &str,
    ) -> Result<Option<Bounded<MIN, MAX>>, Error> {
        self.get(name)
            .map(|value| {
                value
                    .as_i64()
                    .and_then(|number| Bounded::try_from(number).ok())
                    .ok_or_else(|| Error::invalid_parameter(name))
            })
            .transpose()
    }

    fn required_bounded<const MIN: u32, const MAX: u32>(
        &self,
        name: &str,
    ) -> Result<Bounded<MIN, MAX>, Error> {
        let value = self.bounded(name)?;
        self.present(value, name)
    }

    /// `QueueName` of a command that has no other required parameter.
    fn queue_only(&self) -> Result<String, Error> {
        self.require(&["QueueName"])?;
        self.queue()
    }

    fn queue(&self) -> Result<String, Error> {
        self.required_string("QueueName")
    }

    fn queue_name(&self) -> Result<QueueName, Error> {
        let text = self.string("QueueName")?;
        let text = self.present(text, "QueueName")?;
        QueueName::parse(text).ok_or_else(|| {
            Error::new(
                ErrorCode::QueueNameInvalid,
                format!("{text:?} is not a valid queue name"),
            )
        })
    }

    fn attributes(&self) -> Result<Option<AttributePatch>, Error> {
        let members = match self.get("Attributes") {
            None => return Ok(None),
            Some(Value::Object(members)) => members,
            Some(_) => return Err(Error::invalid_parameter("Attributes")),
        };
        if let Some(unknown) = members
            .keys()
            .find(|key| !ATTRIBUTE_NAMES.contains(&key.as_str()))
        {
            return Err(Error::new(
                ErrorCode::InvalidParameterValue,
                format!("unknown attribute {unknown:?}"),
            ));
        }
        let attributes = Params(members);
        Ok(Some(AttributePatch {
            visibility_timeout: attributes.bounded("VisibilityTimeout")?,
            delay: attributes.bounded("DelaySeconds")?,
            retention: attributes.bounded("RetentionSeconds")?,
            fifo: attributes.boolean("Fifo")?,
            content_based_deduplication: attributes.boolean("ContentBasedDeduplication")?,
            dead_letter_queue: attributes.string("DeadLetterQueue")?.map(str::to_owned),
            max_receive_count: attributes.bounded("MaxReceiveCount")?,
        }))
    }
}
