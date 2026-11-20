//! Turns one input line into a typed request, applying the shape, required
//! parameter and parameter value checks (error order steps 1 to 3).

use serde_json::Value;

use crate::attributes::AttributePatch;
use crate::error::{Error, ErrorCode};
use crate::name::QueueName;
use crate::params::{self, Params, integer, string};
use crate::queue::Submission;

#[derive(Debug)]
pub enum Request {
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
        seconds: u64,
    },
    SendMessage {
        queue: String,
        submission: Submission,
    },
    ReceiveMessage {
        queue: String,
        max_messages: u64,
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

impl Request {
    /// Parses `line` (without its terminating newline).
    pub fn parse(line: &[u8]) -> Result<Self, Error> {
        let invalid = |message: String| Error::new(ErrorCode::InvalidRequest, message);
        let text = std::str::from_utf8(line)
            .map_err(|e| invalid(format!("line is not valid UTF-8: {e}")))?;
        let value: Value = serde_json::from_str(text)
            .map_err(|e| invalid(format!("line is not valid JSON: {e}")))?;
        let Value::Object(members) = value else {
            return Err(invalid("line is not a JSON object".into()));
        };
        let Some(Value::String(command)) = members.get("Command") else {
            return Err(invalid("`Command` is missing or not a string".into()));
        };
        let command = command.clone();
        let params = Params::new(members);
        match command.as_str() {
            "CreateQueue" => {
                let [name] = params.required(["QueueName"])?;
                // The name is checked before every other parameter.
                let name = QueueName::parse(string("QueueName", name)?)?;
                let attributes = attributes(&params)?.unwrap_or_default();
                Ok(Self::CreateQueue { name, attributes })
            }
            "SetQueueAttributes" => {
                let [queue, attributes] = params.required(["QueueName", "Attributes"])?;
                Ok(Self::SetQueueAttributes {
                    queue: string("QueueName", queue)?,
                    attributes: AttributePatch::parse(attributes)?,
                })
            }
            "GetQueueAttributes" => Ok(Self::GetQueueAttributes {
                queue: queue_name(&params)?,
            }),
            "ListQueues" => Ok(Self::ListQueues {
                prefix: params
                    .optional("Prefix")
                    .map(|v| string("Prefix", v))
                    .transpose()?,
            }),
            "DeleteQueue" => Ok(Self::DeleteQueue {
                queue: queue_name(&params)?,
            }),
            "PurgeQueue" => Ok(Self::PurgeQueue {
                queue: queue_name(&params)?,
            }),
            "Tick" => {
                let [seconds] = params.required(["Seconds"])?;
                Ok(Self::Tick {
                    seconds: integer("Seconds", seconds, 0..=1_209_600)?,
                })
            }
            "SendMessage" => {
                let [queue, body] = params.required(["QueueName", "Body"])?;
                let queue = string("QueueName", queue)?;
                let body = string("Body", body)?;
                if !(1..=1024).contains(&body.len()) {
                    return Err(params::invalid("Body", "must be 1 to 1024 bytes long"));
                }
                let delay_seconds = params
                    .optional("DelaySeconds")
                    .map(|v| integer("DelaySeconds", v, 0..=900))
                    .transpose()?;
                let group = identifier(&params, "GroupId")?;
                let deduplication_id = identifier(&params, "DeduplicationId")?;
                Ok(Self::SendMessage {
                    queue,
                    submission: Submission {
                        body,
                        delay_seconds,
                        group,
                        deduplication_id,
                    },
                })
            }
            "ReceiveMessage" => {
                let queue = queue_name(&params)?;
                let max_messages = params
                    .optional("MaxMessages")
                    .map(|v| integer("MaxMessages", v, 1..=10))
                    .transpose()?
                    .unwrap_or(1);
                Ok(Self::ReceiveMessage {
                    queue,
                    max_messages,
                    visibility_timeout: visibility_timeout(&params)?,
                })
            }
            "DeleteMessage" => {
                let [queue, receipt_handle] = params.required(["QueueName", "ReceiptHandle"])?;
                Ok(Self::DeleteMessage {
                    queue: string("QueueName", queue)?,
                    receipt_handle: string("ReceiptHandle", receipt_handle)?,
                })
            }
            "ChangeMessageVisibility" => {
                let [queue, receipt_handle, timeout] =
                    params.required(["QueueName", "ReceiptHandle", "VisibilityTimeout"])?;
                Ok(Self::ChangeMessageVisibility {
                    queue: string("QueueName", queue)?,
                    receipt_handle: string("ReceiptHandle", receipt_handle)?,
                    visibility_timeout: integer("VisibilityTimeout", timeout, 0..=43_200)?,
                })
            }
            _ => Err(invalid(format!("unknown command `{command}`"))),
        }
    }
}

fn attributes(params: &Params) -> Result<Option<AttributePatch>, Error> {
    params
        .optional("Attributes")
        .map(AttributePatch::parse)
        .transpose()
}

/// For commands whose only parameter is a required `QueueName`.
fn queue_name(params: &Params) -> Result<String, Error> {
    let [queue] = params.required(["QueueName"])?;
    string("QueueName", queue)
}

/// An optional `GroupId` or `DeduplicationId`: 1 to 128 characters of
/// `A-Z a-z 0-9 _ -`.
fn identifier(params: &Params, name: &str) -> Result<Option<String>, Error> {
    let Some(value) = params.optional(name) else {
        return Ok(None);
    };
    let id = string(name, value)?;
    let is_id_char = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '-';
    if id.is_empty() || id.len() > 128 || !id.chars().all(is_id_char) {
        return Err(params::invalid(
            name,
            "must be 1 to 128 characters of A-Z a-z 0-9 _ -",
        ));
    }
    Ok(Some(id))
}

fn visibility_timeout(params: &Params) -> Result<Option<u64>, Error> {
    params
        .optional("VisibilityTimeout")
        .map(|v| integer("VisibilityTimeout", v, 0..=43_200))
        .transpose()
}
