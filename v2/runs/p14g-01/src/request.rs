//! Parsing of one input line into a typed command, covering the request-shape, required-parameter
//! and parameter-value checks (steps 1 to 3 of the error order).

use serde_json::{Map, Value};

use crate::error::{Error, Problem, ShapeProblem};
use crate::types::{Body, DeduplicationId, GroupId, QueueName, Seconds};

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Attributes {
    pub visibility_timeout: Option<Seconds>,
    pub delay: Option<Seconds>,
    pub retention: Option<Seconds>,
    pub fifo: Option<bool>,
    pub content_based_deduplication: Option<bool>,
    pub dead_letter_queue: Option<String>,
    pub max_receive_count: Option<u32>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct DeadLetterRequest<'a> {
    pub queue: &'a str,
    pub max_receive_count: u32,
}

impl Attributes {
    fn parse(value: &Value) -> Result<Self, Error> {
        let Value::Object(members) = value else {
            return Err(wrong_type("Attributes", "an object"));
        };
        let mut attributes = Self::default();
        for (name, value) in members.iter().filter(|(_, value)| !value.is_null()) {
            match name.as_str() {
                "VisibilityTimeout" => {
                    attributes.visibility_timeout = Some(seconds(name, value, 0, 43_200)?)
                }
                "DelaySeconds" => attributes.delay = Some(seconds(name, value, 0, 900)?),
                "RetentionSeconds" => {
                    attributes.retention = Some(seconds(name, value, 60, 1_209_600)?)
                }
                "Fifo" => attributes.fifo = Some(boolean(name, value)?),
                "ContentBasedDeduplication" => {
                    attributes.content_based_deduplication = Some(boolean(name, value)?)
                }
                "DeadLetterQueue" => {
                    attributes.dead_letter_queue = Some(string(name, value)?.to_owned())
                }
                "MaxReceiveCount" => {
                    attributes.max_receive_count = Some(integer(name, value, 1, 1000)?)
                }
                _ => return Err(Error::invalid_value(name, Problem::UnknownAttribute)),
            }
        }
        Ok(attributes)
    }

    /// The dead-letter configuration of the request: both attributes or neither.
    pub fn dead_letter(&self) -> Result<Option<DeadLetterRequest<'_>>, Error> {
        match (&self.dead_letter_queue, self.max_receive_count) {
            (None, None) => Ok(None),
            (Some(queue), Some(max_receive_count)) => Ok(Some(DeadLetterRequest {
                queue,
                max_receive_count,
            })),
            _ => Err(Error::invalid_value(
                "DeadLetterQueue",
                Problem::IncompleteDeadLetter,
            )),
        }
    }
}

#[derive(Debug)]
pub struct SendParams {
    pub body: Body,
    pub delay: Option<Seconds>,
    pub group: Option<GroupId>,
    pub deduplication_id: Option<DeduplicationId>,
}

#[derive(Debug)]
pub enum Request {
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
        seconds: Seconds,
    },
    SendMessage {
        queue: String,
        send: SendParams,
    },
    ReceiveMessage {
        queue: String,
        max_messages: usize,
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

impl Request {
    pub fn parse(line: &[u8]) -> Result<Self, Error> {
        let text = std::str::from_utf8(line).map_err(ShapeProblem::NotUtf8)?;
        let value: Value = serde_json::from_str(text).map_err(ShapeProblem::NotJson)?;
        let Value::Object(members) = value else {
            return Err(ShapeProblem::NotAnObject.into());
        };
        let Some(Value::String(command)) = members.get("Command") else {
            return Err(ShapeProblem::MissingCommand.into());
        };
        let params = Params(&members);
        match command.as_str() {
            "CreateQueue" => params.create_queue(),
            "SetQueueAttributes" => params.set_queue_attributes(),
            "GetQueueAttributes" => Ok(Self::GetQueueAttributes {
                queue: params.queue()?,
            }),
            "ListQueues" => Ok(Self::ListQueues {
                prefix: params.optional("Prefix", string)?.map(str::to_owned),
            }),
            "DeleteQueue" => Ok(Self::DeleteQueue {
                queue: params.queue()?,
            }),
            "PurgeQueue" => Ok(Self::PurgeQueue {
                queue: params.queue()?,
            }),
            "Tick" => {
                let seconds = params.required("Seconds")?;
                Ok(Self::Tick {
                    seconds: self::seconds("Seconds", seconds, 0, 1_209_600)?,
                })
            }
            "SendMessage" => params.send_message(),
            "ReceiveMessage" => params.receive_message(),
            "DeleteMessage" => {
                let (queue, handle) = (
                    params.required("QueueName")?,
                    params.required("ReceiptHandle")?,
                );
                Ok(Self::DeleteMessage {
                    queue: string("QueueName", queue)?.to_owned(),
                    receipt_handle: string("ReceiptHandle", handle)?.to_owned(),
                })
            }
            "ChangeMessageVisibility" => params.change_message_visibility(),
            other => Err(ShapeProblem::UnknownCommand(other.to_owned()).into()),
        }
    }
}

/// The members of a command; a `null` member counts as absent.
struct Params<'a>(&'a Map<String, Value>);

impl<'a> Params<'a> {
    fn get(&self, name: &str) -> Option<&'a Value> {
        self.0.get(name).filter(|value| !value.is_null())
    }

    fn required(&self, name: &'static str) -> Result<&'a Value, Error> {
        self.get(name).ok_or(Error::MissingParameter { name })
    }

    fn optional<T>(
        &self,
        name: &str,
        read: impl FnOnce(&str, &'a Value) -> Result<T, Error>,
    ) -> Result<Option<T>, Error> {
        self.get(name).map(|value| read(name, value)).transpose()
    }

    fn queue(&self) -> Result<String, Error> {
        string("QueueName", self.required("QueueName")?).map(str::to_owned)
    }

    fn create_queue(&self) -> Result<Request, Error> {
        let name = self.required("QueueName")?;
        let name = QueueName::parse(string("QueueName", name)?)?;
        let attributes = self.optional("Attributes", |_, value| Attributes::parse(value))?;
        Ok(Request::CreateQueue {
            name,
            attributes: attributes.unwrap_or_default(),
        })
    }

    fn set_queue_attributes(&self) -> Result<Request, Error> {
        let (queue, attributes) = (self.required("QueueName")?, self.required("Attributes")?);
        Ok(Request::SetQueueAttributes {
            queue: string("QueueName", queue)?.to_owned(),
            attributes: Attributes::parse(attributes)?,
        })
    }

    fn send_message(&self) -> Result<Request, Error> {
        let (queue, body) = (self.required("QueueName")?, self.required("Body")?);
        let queue = string("QueueName", queue)?.to_owned();
        let body = Body::parse("Body", string("Body", body)?)?;
        let delay = self.optional("DelaySeconds", |name, value| seconds(name, value, 0, 900))?;
        let group = self.optional("GroupId", |name, value| {
            GroupId::parse(name, string(name, value)?)
        })?;
        let deduplication_id = self.optional("DeduplicationId", |name, value| {
            DeduplicationId::parse(name, string(name, value)?)
        })?;
        Ok(Request::SendMessage {
            queue,
            send: SendParams {
                body,
                delay,
                group,
                deduplication_id,
            },
        })
    }

    fn receive_message(&self) -> Result<Request, Error> {
        let queue = self.queue()?;
        let max_messages = self
            .optional("MaxMessages", |name, value| {
                integer::<usize>(name, value, 1, 10)
            })?
            .unwrap_or(1);
        let visibility_timeout = self.optional("VisibilityTimeout", |name, value| {
            seconds(name, value, 0, 43_200)
        })?;
        Ok(Request::ReceiveMessage {
            queue,
            max_messages,
            visibility_timeout,
        })
    }

    fn change_message_visibility(&self) -> Result<Request, Error> {
        let (queue, handle, timeout) = (
            self.required("QueueName")?,
            self.required("ReceiptHandle")?,
            self.required("VisibilityTimeout")?,
        );
        Ok(Request::ChangeMessageVisibility {
            queue: string("QueueName", queue)?.to_owned(),
            receipt_handle: string("ReceiptHandle", handle)?.to_owned(),
            visibility_timeout: seconds("VisibilityTimeout", timeout, 0, 43_200)?,
        })
    }
}

fn wrong_type(name: &str, expected: &'static str) -> Error {
    Error::invalid_value(name, Problem::WrongType { expected })
}

fn string<'a>(name: &str, value: &'a Value) -> Result<&'a str, Error> {
    value.as_str().ok_or_else(|| wrong_type(name, "a string"))
}

fn boolean(name: &str, value: &Value) -> Result<bool, Error> {
    value.as_bool().ok_or_else(|| wrong_type(name, "a boolean"))
}

/// An integer literal without fraction or exponent, within `min..=max`.
fn integer<T: TryFrom<i64>>(name: &str, value: &Value, min: i64, max: i64) -> Result<T, Error> {
    let Value::Number(number) = value else {
        return Err(wrong_type(name, "an integer"));
    };
    // With arbitrary_precision the number keeps its source text, so `30.0` and `3e1` do not parse.
    let parsed: i64 = number
        .to_string()
        .parse()
        .map_err(|_| wrong_type(name, "an integer"))?;
    let out_of_range = || Error::invalid_value(name, Problem::OutOfRange { min, max });
    if !(min..=max).contains(&parsed) {
        return Err(out_of_range());
    }
    T::try_from(parsed).map_err(|_| out_of_range())
}

fn seconds(name: &str, value: &Value, min: i64, max: i64) -> Result<Seconds, Error> {
    integer::<u64>(name, value, min, max).map(Seconds::new)
}
