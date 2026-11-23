//! Parsing of one input line into a command (SPEC sections 1 and 2, steps 1 to 3).

use serde_json::Value;

use crate::attributes::AttributeUpdate;
use crate::error::{Code, Error, Result};
use crate::params::{Params, as_int, as_string};
use crate::types::{DedupId, GroupId, QueueName, Seconds};

const MAX_BODY_BYTES: usize = 1024;

#[derive(Debug)]
pub struct SendMessage {
    pub queue: String,
    pub body: String,
    pub delay: Option<Seconds>,
    pub group: Option<GroupId>,
    pub dedup: Option<DedupId>,
}

#[derive(Debug)]
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
        seconds: Seconds,
    },
    SendMessage(SendMessage),
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

impl Command {
    pub fn parse(line: &[u8]) -> Result<Self> {
        let text = std::str::from_utf8(line)
            .map_err(|_| Error::new(Code::InvalidRequest, "line is not valid UTF-8"))?;
        let Value::Object(members) = serde_json::from_str(text)
            .map_err(|e| Error::new(Code::InvalidRequest, format!("line is not JSON: {e}")))?
        else {
            return Err(Error::new(
                Code::InvalidRequest,
                "line is not a JSON object",
            ));
        };
        let params = Params(&members);
        let name = params
            .get("Command")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::new(Code::InvalidRequest, "Command must be a string"))?;
        match name {
            "CreateQueue" => Self::create_queue(&params),
            "SetQueueAttributes" => Self::set_queue_attributes(&params),
            "GetQueueAttributes" => Ok(Command::GetQueueAttributes {
                queue: queue_of(&params)?,
            }),
            "ListQueues" => Ok(Command::ListQueues {
                prefix: params.optional("Prefix", |v| as_string("Prefix", v).map(str::to_owned))?,
            }),
            "DeleteQueue" => Ok(Command::DeleteQueue {
                queue: queue_of(&params)?,
            }),
            "PurgeQueue" => Ok(Command::PurgeQueue {
                queue: queue_of(&params)?,
            }),
            "Tick" => {
                let seconds = params.require("Seconds")?;
                Ok(Command::Tick {
                    seconds: Seconds::new(as_int("Seconds", seconds, 0, 1_209_600)?),
                })
            }
            "SendMessage" => Self::send_message(&params),
            "ReceiveMessage" => Self::receive_message(&params),
            "DeleteMessage" => {
                let queue = params.require("QueueName")?;
                let handle = params.require("ReceiptHandle")?;
                Ok(Command::DeleteMessage {
                    queue: as_string("QueueName", queue)?.to_owned(),
                    receipt_handle: as_string("ReceiptHandle", handle)?.to_owned(),
                })
            }
            "ChangeMessageVisibility" => {
                let queue = params.require("QueueName")?;
                let handle = params.require("ReceiptHandle")?;
                let timeout = params.require("VisibilityTimeout")?;
                Ok(Command::ChangeMessageVisibility {
                    queue: as_string("QueueName", queue)?.to_owned(),
                    receipt_handle: as_string("ReceiptHandle", handle)?.to_owned(),
                    visibility_timeout: visibility_timeout(timeout)?,
                })
            }
            other => Err(Error::new(
                Code::InvalidRequest,
                format!("unknown command {other:?}"),
            )),
        }
    }

    fn create_queue(params: &Params) -> Result<Self> {
        let name = params.require("QueueName")?;
        // The name is checked before every other parameter, with its own error code.
        let name = QueueName::parse(as_string("QueueName", name)?)?;
        let attributes = params
            .optional("Attributes", AttributeUpdate::parse)?
            .unwrap_or_default();
        Ok(Command::CreateQueue { name, attributes })
    }

    fn set_queue_attributes(params: &Params) -> Result<Self> {
        let queue = params.require("QueueName")?;
        let attributes = params.require("Attributes")?;
        Ok(Command::SetQueueAttributes {
            queue: as_string("QueueName", queue)?.to_owned(),
            attributes: AttributeUpdate::parse(attributes)?,
        })
    }

    fn send_message(params: &Params) -> Result<Self> {
        let queue = params.require("QueueName")?;
        let body = params.require("Body")?;
        let queue = as_string("QueueName", queue)?.to_owned();
        let body = as_string("Body", body)?;
        if !(1..=MAX_BODY_BYTES).contains(&body.len()) {
            return Err(Error::invalid_value(format!(
                "Body must be 1 to {MAX_BODY_BYTES} bytes"
            )));
        }
        let delay = params.optional("DelaySeconds", |v| {
            as_int("DelaySeconds", v, 0, 900).map(Seconds::new)
        })?;
        let group = params.optional("GroupId", |v| GroupId::parse(as_string("GroupId", v)?))?;
        let dedup = params.optional("DeduplicationId", |v| {
            DedupId::parse(as_string("DeduplicationId", v)?)
        })?;
        Ok(Command::SendMessage(SendMessage {
            queue,
            body: body.to_owned(),
            delay,
            group,
            dedup,
        }))
    }

    fn receive_message(params: &Params) -> Result<Self> {
        let queue = queue_of(params)?;
        let max_messages = params
            .optional("MaxMessages", |v| as_int("MaxMessages", v, 1, 10))?
            .unwrap_or(1);
        let visibility_timeout = params.optional("VisibilityTimeout", visibility_timeout)?;
        Ok(Command::ReceiveMessage {
            queue,
            max_messages,
            visibility_timeout,
        })
    }
}

fn queue_of(params: &Params) -> Result<String> {
    let queue = params.require("QueueName")?;
    Ok(as_string("QueueName", queue)?.to_owned())
}

fn visibility_timeout(value: &Value) -> Result<Seconds> {
    as_int("VisibilityTimeout", value, 0, 43_200).map(Seconds::new)
}
