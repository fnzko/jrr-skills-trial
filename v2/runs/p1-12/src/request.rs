use serde_json::{Map, Value};

use crate::attributes::AttributeUpdate;
use crate::error::{Error, ErrorCode};
use crate::params::{Params, invalid};

/// A parsed command. Parsing covers the request shape, required parameters and
/// parameter values (checks 1 to 3 of the error order); everything that needs
/// queue state is left to the broker.
#[derive(Debug)]
pub(crate) enum Request {
    CreateQueue {
        name: String,
        attributes: AttributeUpdate,
    },
    SetQueueAttributes {
        queue: String,
        attributes: AttributeUpdate,
    },
    ListQueues {
        prefix: Option<String>,
    },
    DeleteQueue {
        queue: String,
    },
    Tick {
        seconds: u32,
    },
    GetQueueAttributes {
        queue: String,
    },
    PurgeQueue {
        queue: String,
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
    ReceiveMessage {
        queue: String,
        max_messages: u32,
        visibility_timeout: Option<u32>,
    },
    SendMessage {
        queue: String,
        body: String,
        delay_seconds: Option<u32>,
        group_id: Option<String>,
        deduplication_id: Option<String>,
    },
}

impl Request {
    pub fn parse(line: &[u8]) -> Result<Self, Error> {
        let (command, members) = split_envelope(line)?;
        let p = Params::new(&members);
        match command.as_str() {
            "CreateQueue" => {
                p.require(&["QueueName"])?;
                let name = string(&p, "QueueName")?;
                if !is_valid_queue_name(&name) {
                    return Err(Error::new(
                        ErrorCode::QueueNameInvalid,
                        format!("`{name}` breaks the queue naming rules"),
                    ));
                }
                let attributes = match p.get("Attributes") {
                    Some(value) => AttributeUpdate::parse(value)?,
                    None => AttributeUpdate::default(),
                };
                Ok(Self::CreateQueue { name, attributes })
            }
            "SetQueueAttributes" => {
                p.require(&["QueueName", "Attributes"])?;
                let queue = string(&p, "QueueName")?;
                let attributes = AttributeUpdate::parse(&members["Attributes"])?;
                Ok(Self::SetQueueAttributes { queue, attributes })
            }
            "ListQueues" => Ok(Self::ListQueues {
                prefix: p.string("Prefix")?.map(str::to_owned),
            }),
            "DeleteQueue" => Ok(Self::DeleteQueue {
                queue: queue_name(&p)?,
            }),
            "GetQueueAttributes" => Ok(Self::GetQueueAttributes {
                queue: queue_name(&p)?,
            }),
            "PurgeQueue" => Ok(Self::PurgeQueue {
                queue: queue_name(&p)?,
            }),
            "DeleteMessage" => {
                p.require(&["QueueName", "ReceiptHandle"])?;
                Ok(Self::DeleteMessage {
                    queue: string(&p, "QueueName")?,
                    receipt_handle: string(&p, "ReceiptHandle")?,
                })
            }
            "ChangeMessageVisibility" => {
                p.require(&["QueueName", "ReceiptHandle", "VisibilityTimeout"])?;
                let queue = string(&p, "QueueName")?;
                let receipt_handle = string(&p, "ReceiptHandle")?;
                let visibility_timeout = p.integer("VisibilityTimeout", 0..=43_200)?;
                Ok(Self::ChangeMessageVisibility {
                    queue,
                    receipt_handle,
                    visibility_timeout: visibility_timeout
                        .expect("`VisibilityTimeout` was checked to be present"),
                })
            }
            "ReceiveMessage" => {
                p.require(&["QueueName"])?;
                Ok(Self::ReceiveMessage {
                    queue: string(&p, "QueueName")?,
                    max_messages: p.integer("MaxMessages", 1..=10)?.unwrap_or(1),
                    visibility_timeout: p.integer("VisibilityTimeout", 0..=43_200)?,
                })
            }
            "SendMessage" => {
                p.require(&["QueueName", "Body"])?;
                let queue = string(&p, "QueueName")?;
                let body = string(&p, "Body")?;
                if !(1..=MAX_BODY_BYTES).contains(&body.len()) {
                    return Err(invalid("Body", "1 to 1024 bytes long"));
                }
                Ok(Self::SendMessage {
                    queue,
                    body,
                    delay_seconds: p.integer("DelaySeconds", 0..=900)?,
                    group_id: identifier(&p, "GroupId")?,
                    deduplication_id: identifier(&p, "DeduplicationId")?,
                })
            }
            "Tick" => {
                p.require(&["Seconds"])?;
                let seconds = p.integer("Seconds", 0..=1_209_600)?;
                Ok(Self::Tick {
                    seconds: seconds.expect("`Seconds` was checked to be present"),
                })
            }
            other => Err(Error::new(
                ErrorCode::InvalidRequest,
                format!("unknown command `{other}`"),
            )),
        }
    }
}

/// Splits a line into its `Command` and the object's other members.
fn split_envelope(line: &[u8]) -> Result<(String, Map<String, Value>), Error> {
    let invalid_request = |message: String| Error::new(ErrorCode::InvalidRequest, message);

    let text = std::str::from_utf8(line)
        .map_err(|e| invalid_request(format!("line is not valid UTF-8: {e}")))?;
    let value: Value = serde_json::from_str(text)
        .map_err(|e| invalid_request(format!("line is not valid JSON: {e}")))?;
    let Value::Object(mut members) = value else {
        return Err(invalid_request("line is not a JSON object".into()));
    };
    let Some(Value::String(command)) = members.remove("Command") else {
        return Err(invalid_request(
            "`Command` is missing or not a string".into(),
        ));
    };
    Ok((command, members))
}

/// The `QueueName` of a command that takes nothing else.
fn queue_name(p: &Params<'_>) -> Result<String, Error> {
    p.require(&["QueueName"])?;
    string(p, "QueueName")
}

/// A `GroupId` or `DeduplicationId`: 1 to 128 characters of the queue-name
/// alphabet.
fn identifier(p: &Params<'_>, name: &str) -> Result<Option<String>, Error> {
    p.string(name)?
        .map(|id| {
            if (1..=128).contains(&id.len()) && id.chars().all(is_name_char) {
                Ok(id.to_owned())
            } else {
                Err(invalid(name, "1 to 128 characters of A-Z a-z 0-9 _ -"))
            }
        })
        .transpose()
}

/// A parameter that [`Params::require`] already found present.
fn string(p: &Params<'_>, name: &str) -> Result<String, Error> {
    p.string(name)?
        .map(str::to_owned)
        .ok_or_else(|| invalid(name, "a string"))
}

fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

const MAX_BODY_BYTES: usize = 1024;

const FIFO_SUFFIX: &str = ".fifo";

pub(crate) fn has_fifo_suffix(name: &str) -> bool {
    name.ends_with(FIFO_SUFFIX)
}

fn is_valid_queue_name(name: &str) -> bool {
    let (stem, max_len) = match name.strip_suffix(FIFO_SUFFIX) {
        Some(stem) => (stem, 75),
        None => (name, 80),
    };
    (1..=max_len).contains(&stem.len()) && stem.chars().all(is_name_char)
}
