//! Turning a request line into a checked command.

use std::ops::RangeInclusive;

use serde_json::{Map, Value};

use crate::attributes::AttributeUpdates;
use crate::attributes::{DELAY_SECONDS, VISIBILITY_TIMEOUT};
use crate::error::{Error, ErrorCode};
use crate::ids::{GroupId, QueueName, is_token};
use crate::params::{Params, integer, string};
use crate::queue::MessageDraft;
use crate::time::Seconds;

const TICK_SECONDS: RangeInclusive<u32> = 0..=1_209_600;
const BODY_BYTES: RangeInclusive<usize> = 1..=1024;
const MAX_MESSAGES: RangeInclusive<u32> = 1..=10;
const DEFAULT_MAX_MESSAGES: u32 = 1;

/// A command whose request shape, required parameters and parameter values
/// have all been checked, in that order.
#[derive(Debug)]
pub enum Command {
    CreateQueue {
        name: QueueName,
        attributes: AttributeUpdates,
    },
    SetQueueAttributes {
        queue: String,
        attributes: AttributeUpdates,
    },
    GetQueueAttributes {
        queue: String,
    },
    ListQueues {
        prefix: String,
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
        draft: MessageDraft,
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

fn invalid_request(message: impl Into<String>) -> Error {
    Error::new(ErrorCode::InvalidRequest, message)
}

fn attribute_object(value: &Value) -> Result<&Map<String, Value>, Error> {
    value
        .as_object()
        .ok_or_else(|| Error::invalid("Attributes", "must be an object"))
}

impl Command {
    /// Parses one input line.
    pub fn parse(line: &[u8]) -> Result<Self, Error> {
        let request: Map<String, Value> = serde_json::from_slice(line)
            .map_err(|source| invalid_request(format!("line is not a JSON object: {source}")))?;
        let Some(Value::String(name)) = request.get("Command") else {
            return Err(invalid_request("member Command must be a string"));
        };
        let params = Params(&request);
        match name.as_str() {
            "CreateQueue" => Self::create_queue(&params),
            "SetQueueAttributes" => {
                let queue = params.required("QueueName")?;
                let attributes = params.required("Attributes")?;
                Ok(Self::SetQueueAttributes {
                    queue: string("QueueName", queue)?.to_owned(),
                    attributes: AttributeUpdates::parse(attribute_object(attributes)?)?,
                })
            }
            "GetQueueAttributes" => Ok(Self::GetQueueAttributes {
                queue: Self::queue_only(&params)?,
            }),
            "ListQueues" => {
                let prefix = params
                    .optional("Prefix")
                    .map(|value| string("Prefix", value))
                    .transpose()?;
                Ok(Self::ListQueues {
                    prefix: prefix.unwrap_or_default().to_owned(),
                })
            }
            "DeleteQueue" => Ok(Self::DeleteQueue {
                queue: Self::queue_only(&params)?,
            }),
            "PurgeQueue" => Ok(Self::PurgeQueue {
                queue: Self::queue_only(&params)?,
            }),
            "Tick" => {
                let seconds = params.required("Seconds")?;
                Ok(Self::Tick {
                    seconds: Seconds(integer("Seconds", seconds, &TICK_SECONDS)?),
                })
            }
            "SendMessage" => Self::send_message(&params),
            "ReceiveMessage" => {
                let queue = Self::queue_only(&params)?;
                let max_messages = params
                    .optional("MaxMessages")
                    .map(|value| integer("MaxMessages", value, &MAX_MESSAGES))
                    .transpose()?;
                Ok(Self::ReceiveMessage {
                    queue,
                    max_messages: max_messages.unwrap_or(DEFAULT_MAX_MESSAGES),
                    visibility_timeout: Self::optional_visibility_timeout(&params)?,
                })
            }
            "DeleteMessage" => {
                let (queue, receipt_handle) = Self::queue_and_handle(&params)?;
                Ok(Self::DeleteMessage {
                    queue,
                    receipt_handle,
                })
            }
            "ChangeMessageVisibility" => {
                let queue = params.required("QueueName")?;
                let handle = params.required("ReceiptHandle")?;
                let timeout = params.required("VisibilityTimeout")?;
                Ok(Self::ChangeMessageVisibility {
                    queue: string("QueueName", queue)?.to_owned(),
                    receipt_handle: string("ReceiptHandle", handle)?.to_owned(),
                    visibility_timeout: Seconds(integer(
                        "VisibilityTimeout",
                        timeout,
                        &VISIBILITY_TIMEOUT,
                    )?),
                })
            }
            other => Err(invalid_request(format!("unknown command {other:?}"))),
        }
    }

    fn queue_only(params: &Params<'_>) -> Result<String, Error> {
        let queue = params.required("QueueName")?;
        Ok(string("QueueName", queue)?.to_owned())
    }

    fn queue_and_handle(params: &Params<'_>) -> Result<(String, String), Error> {
        let queue = params.required("QueueName")?;
        let handle = params.required("ReceiptHandle")?;
        Ok((
            string("QueueName", queue)?.to_owned(),
            string("ReceiptHandle", handle)?.to_owned(),
        ))
    }

    fn optional_visibility_timeout(params: &Params<'_>) -> Result<Option<Seconds>, Error> {
        params
            .optional("VisibilityTimeout")
            .map(|value| integer("VisibilityTimeout", value, &VISIBILITY_TIMEOUT).map(Seconds))
            .transpose()
    }

    fn send_message(params: &Params<'_>) -> Result<Self, Error> {
        let queue = params.required("QueueName")?;
        let body = params.required("Body")?;
        let queue = string("QueueName", queue)?.to_owned();
        let body = string("Body", body)?;
        if !BODY_BYTES.contains(&body.len()) {
            return Err(Error::invalid("Body", "must be 1 to 1024 bytes long"));
        }
        let delay = params
            .optional("DelaySeconds")
            .map(|value| integer("DelaySeconds", value, &DELAY_SECONDS).map(Seconds))
            .transpose()?;
        let group_id = params
            .optional("GroupId")
            .map(|value| {
                GroupId::parse(string("GroupId", value)?).ok_or_else(|| {
                    Error::invalid("GroupId", "must be 1 to 128 characters of A-Z a-z 0-9 _ -")
                })
            })
            .transpose()?;
        let deduplication_id = params
            .optional("DeduplicationId")
            .map(|value| {
                let id = string("DeduplicationId", value)?;
                if is_token(id) {
                    Ok(id.to_owned())
                } else {
                    Err(Error::invalid(
                        "DeduplicationId",
                        "must be 1 to 128 characters of A-Z a-z 0-9 _ -",
                    ))
                }
            })
            .transpose()?;
        Ok(Self::SendMessage {
            queue,
            draft: MessageDraft {
                body: body.to_owned(),
                delay,
                group_id,
                deduplication_id,
            },
        })
    }

    fn create_queue(params: &Params<'_>) -> Result<Self, Error> {
        let name = params.required("QueueName")?;
        // The name is checked before the other parameters, so a bad name wins
        // over a malformed `Attributes`.
        let name = QueueName::parse(string("QueueName", name)?)?;
        let attributes = params
            .optional("Attributes")
            .map(|value| AttributeUpdates::parse(attribute_object(value)?))
            .transpose()?
            .unwrap_or_default();
        Ok(Self::CreateQueue { name, attributes })
    }
}
