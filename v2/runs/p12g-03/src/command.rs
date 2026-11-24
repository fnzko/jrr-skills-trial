use serde_json::{Map, Value};

use crate::attributes::Attributes;
use crate::error::{Error, ErrorCode};
use crate::params::Members;
use crate::types::{Identifier, QueueName, Seconds};

/// A request that passed the shape, required-parameter and value checks.
#[derive(Debug)]
pub enum Command<'a> {
    CreateQueue {
        name: QueueName,
        attributes: Attributes<'a>,
    },
    SetQueueAttributes {
        queue: &'a str,
        attributes: Attributes<'a>,
    },
    ListQueues {
        prefix: Option<&'a str>,
    },
    DeleteQueue {
        queue: &'a str,
    },
    Tick {
        seconds: Seconds,
    },
    GetQueueAttributes {
        queue: &'a str,
    },
    PurgeQueue {
        queue: &'a str,
    },
    SendMessage(SendRequest<'a>),
    ReceiveMessage {
        queue: &'a str,
        max_messages: usize,
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

#[derive(Debug)]
pub struct SendRequest<'a> {
    pub queue: &'a str,
    pub body: &'a str,
    pub delay: Option<Seconds>,
    pub group: Option<Identifier<'a>>,
    pub deduplication_id: Option<Identifier<'a>>,
}

impl<'a> Command<'a> {
    pub fn parse(members: &'a Map<String, Value>) -> Result<Self, Error> {
        let Some(Value::String(name)) = members.get("Command") else {
            return Err(Error::new(
                ErrorCode::InvalidRequest,
                "Command must be a string member",
            ));
        };
        let params = Members::new(members);
        match name.as_str() {
            "CreateQueue" => {
                let name = params.required("QueueName")?;
                let attributes = params.optional("Attributes");
                let name = QueueName::parse(name.string()?)?;
                let attributes = attributes
                    .map(|param| param.object().and_then(Attributes::parse))
                    .transpose()?
                    .unwrap_or_default();
                Ok(Command::CreateQueue { name, attributes })
            }
            "SetQueueAttributes" => {
                let queue = params.required("QueueName")?;
                let attributes = params.required("Attributes")?;
                Ok(Command::SetQueueAttributes {
                    queue: queue.string()?,
                    attributes: Attributes::parse(attributes.object()?)?,
                })
            }
            "ListQueues" => Ok(Command::ListQueues {
                prefix: params.optional("Prefix").map(|p| p.string()).transpose()?,
            }),
            "DeleteQueue" => Ok(Command::DeleteQueue {
                queue: params.required("QueueName")?.string()?,
            }),
            "Tick" => Ok(Command::Tick {
                seconds: params.required("Seconds")?.seconds(0..=1_209_600)?,
            }),
            "GetQueueAttributes" => Ok(Command::GetQueueAttributes {
                queue: params.required("QueueName")?.string()?,
            }),
            "PurgeQueue" => Ok(Command::PurgeQueue {
                queue: params.required("QueueName")?.string()?,
            }),
            "SendMessage" => {
                let queue = params.required("QueueName")?;
                let body = params.required("Body")?;
                let delay = params.optional("DelaySeconds");
                let group = params.optional("GroupId");
                let deduplication_id = params.optional("DeduplicationId");
                Ok(Command::SendMessage(SendRequest {
                    queue: queue.string()?,
                    body: body.text(1..=1024)?,
                    delay: delay.map(|p| p.seconds(0..=900)).transpose()?,
                    group: group.map(|p| p.identifier()).transpose()?,
                    deduplication_id: deduplication_id.map(|p| p.identifier()).transpose()?,
                }))
            }
            "ReceiveMessage" => {
                let queue = params.required("QueueName")?;
                let max_messages = params.optional("MaxMessages");
                let visibility = params.optional("VisibilityTimeout");
                Ok(Command::ReceiveMessage {
                    queue: queue.string()?,
                    max_messages: max_messages
                        .map(|p| p.bounded(1..=10))
                        .transpose()?
                        .unwrap_or(1),
                    visibility_timeout: visibility.map(|p| p.seconds(0..=43_200)).transpose()?,
                })
            }
            "DeleteMessage" => {
                let queue = params.required("QueueName")?;
                let receipt_handle = params.required("ReceiptHandle")?;
                Ok(Command::DeleteMessage {
                    queue: queue.string()?,
                    receipt_handle: receipt_handle.string()?,
                })
            }
            "ChangeMessageVisibility" => {
                let queue = params.required("QueueName")?;
                let receipt_handle = params.required("ReceiptHandle")?;
                let visibility = params.required("VisibilityTimeout")?;
                Ok(Command::ChangeMessageVisibility {
                    queue: queue.string()?,
                    receipt_handle: receipt_handle.string()?,
                    visibility_timeout: visibility.seconds(0..=43_200)?,
                })
            }
            other => Err(Error::new(
                ErrorCode::InvalidRequest,
                format!("unknown command {other:?}"),
            )),
        }
    }
}
