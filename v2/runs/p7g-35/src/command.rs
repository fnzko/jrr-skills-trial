//! Turns a request line into a validated [`Command`].
//!
//! Parsing performs steps 1 to 3 of the check order in SPEC section 2: request
//! shape, required parameters, parameter values. Everything that needs state
//! (queue lookup and the per-command rules) happens in the service.

use serde_json::Value;

use crate::error::{Error, ErrorCode, Result};
use crate::params::{invalid, Params};
use crate::queue::{is_name_char, is_valid_queue_name};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    CreateQueue {
        name: String,
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
    SendMessage(SendMessage),
    ReceiveMessage {
        queue: String,
        max_messages: Option<usize>,
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
        seconds: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendMessage {
    pub queue: String,
    pub body: String,
    pub delay_seconds: Option<u32>,
    pub group_id: Option<String>,
    pub deduplication_id: Option<String>,
}

/// The queue attributes given in a request; absent ones are `None`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AttributeUpdate {
    pub visibility_timeout: Option<u32>,
    pub delay_seconds: Option<u32>,
    pub retention_seconds: Option<u64>,
    pub fifo: Option<bool>,
    pub content_based_deduplication: Option<bool>,
    pub dead_letter_queue: Option<String>,
    pub max_receive_count: Option<u32>,
}

const MAX_VISIBILITY_TIMEOUT: u64 = 43_200;
pub const MAX_TICK_SECONDS: u64 = 1_209_600;

fn invalid_request(message: impl Into<String>) -> Error {
    Error::new(ErrorCode::InvalidRequest, message)
}

pub fn parse(line: &[u8]) -> Result<Command> {
    let text = std::str::from_utf8(line).map_err(|_| invalid_request("line is not valid UTF-8"))?;
    let value: Value =
        serde_json::from_str(text).map_err(|e| invalid_request(format!("invalid JSON: {e}")))?;
    let Value::Object(members) = value else {
        return Err(invalid_request("request is not a JSON object"));
    };
    let params = Params::new(&members);
    let name = match members.get("Command") {
        Some(Value::String(name)) => name.as_str(),
        _ => return Err(invalid_request("Command must be a string")),
    };
    match name {
        "CreateQueue" => parse_create_queue(&params),
        "SetQueueAttributes" => parse_set_queue_attributes(&params),
        "GetQueueAttributes" => {
            queue_only(&params).map(|queue| Command::GetQueueAttributes { queue })
        }
        "ListQueues" => Ok(Command::ListQueues {
            prefix: params.string("Prefix")?.map(str::to_owned),
        }),
        "DeleteQueue" => queue_only(&params).map(|queue| Command::DeleteQueue { queue }),
        "PurgeQueue" => queue_only(&params).map(|queue| Command::PurgeQueue { queue }),
        "SendMessage" => parse_send_message(&params),
        "ReceiveMessage" => parse_receive_message(&params),
        "DeleteMessage" => parse_delete_message(&params),
        "ChangeMessageVisibility" => parse_change_message_visibility(&params),
        "Tick" => parse_tick(&params),
        other => Err(invalid_request(format!("unknown command {other:?}"))),
    }
}

fn parse_tick(p: &Params) -> Result<Command> {
    p.require_all(&["Seconds"])?;
    let seconds = p.required_integer("Seconds", 0, MAX_TICK_SECONDS)?;
    Ok(Command::Tick { seconds })
}

/// Commands whose only parameter is `QueueName`.
fn queue_only(p: &Params) -> Result<String> {
    p.require_all(&["QueueName"])?;
    queue_name(p)
}

fn queue_name(p: &Params) -> Result<String> {
    Ok(p.required_string("QueueName")?.to_owned())
}

fn parse_create_queue(p: &Params) -> Result<Command> {
    p.require_all(&["QueueName"])?;
    // The name is checked before everything else, with its own error code.
    let name = queue_name(p)?;
    if !is_valid_queue_name(&name) {
        return Err(Error::new(
            ErrorCode::QueueNameInvalid,
            format!("{name:?} is not a valid queue name"),
        ));
    }
    let attributes = match p.get("Attributes") {
        Some(value) => parse_attributes(value)?,
        None => AttributeUpdate::default(),
    };
    Ok(Command::CreateQueue { name, attributes })
}

fn parse_set_queue_attributes(p: &Params) -> Result<Command> {
    p.require_all(&["QueueName", "Attributes"])?;
    let queue = queue_name(p)?;
    let attributes = parse_attributes(p.required("Attributes")?)?;
    Ok(Command::SetQueueAttributes { queue, attributes })
}

const ATTRIBUTE_NAMES: [&str; 7] = [
    "VisibilityTimeout",
    "DelaySeconds",
    "RetentionSeconds",
    "Fifo",
    "ContentBasedDeduplication",
    "DeadLetterQueue",
    "MaxReceiveCount",
];

fn parse_attributes(value: &Value) -> Result<AttributeUpdate> {
    let Value::Object(members) = value else {
        return Err(invalid("Attributes", "must be an object"));
    };
    // An unknown name is an error even when its value is null: the rule is
    // about the names that may appear, not about their values.
    if let Some(unknown) = members
        .keys()
        .find(|k| !ATTRIBUTE_NAMES.contains(&k.as_str()))
    {
        return Err(invalid(
            "Attributes",
            format!("has unknown attribute {unknown:?}"),
        ));
    }
    let a = Params::new(members);
    Ok(AttributeUpdate {
        visibility_timeout: a
            .integer("VisibilityTimeout", 0, MAX_VISIBILITY_TIMEOUT)?
            .map(|v| v as u32),
        delay_seconds: a.integer("DelaySeconds", 0, 900)?.map(|v| v as u32),
        retention_seconds: a.integer("RetentionSeconds", 60, 1_209_600)?,
        fifo: a.boolean("Fifo")?,
        content_based_deduplication: a.boolean("ContentBasedDeduplication")?,
        dead_letter_queue: a.string("DeadLetterQueue")?.map(str::to_owned),
        max_receive_count: a.integer("MaxReceiveCount", 1, 1000)?.map(|v| v as u32),
    })
}

const MAX_BODY_BYTES: usize = 1024;
const MAX_ID_CHARS: usize = 128;

fn parse_send_message(p: &Params) -> Result<Command> {
    p.require_all(&["QueueName", "Body"])?;
    let queue = queue_name(p)?;
    let body = p
        .required_string_with_len("Body", MAX_BODY_BYTES)?
        .to_owned();
    let delay_seconds = p.integer("DelaySeconds", 0, 900)?.map(|v| v as u32);
    let group_id = message_id_string(p, "GroupId")?;
    let deduplication_id = message_id_string(p, "DeduplicationId")?;
    Ok(Command::SendMessage(SendMessage {
        queue,
        body,
        delay_seconds,
        group_id,
        deduplication_id,
    }))
}

/// A `GroupId` or `DeduplicationId`: 1 to 128 characters of `A-Z a-z 0-9 _ -`.
fn message_id_string(p: &Params, key: &str) -> Result<Option<String>> {
    let Some(value) = p.string(key)? else {
        return Ok(None);
    };
    if (1..=MAX_ID_CHARS).contains(&value.len()) && value.chars().all(is_name_char) {
        Ok(Some(value.to_owned()))
    } else {
        Err(invalid(
            key,
            format!("must be 1 to {MAX_ID_CHARS} characters of A-Z a-z 0-9 _ -"),
        ))
    }
}

fn parse_receive_message(p: &Params) -> Result<Command> {
    p.require_all(&["QueueName"])?;
    let queue = queue_name(p)?;
    let max_messages = p.integer("MaxMessages", 1, 10)?.map(|v| v as usize);
    let visibility_timeout = p
        .integer("VisibilityTimeout", 0, MAX_VISIBILITY_TIMEOUT)?
        .map(|v| v as u32);
    Ok(Command::ReceiveMessage {
        queue,
        max_messages,
        visibility_timeout,
    })
}

fn parse_delete_message(p: &Params) -> Result<Command> {
    p.require_all(&["QueueName", "ReceiptHandle"])?;
    Ok(Command::DeleteMessage {
        queue: queue_name(p)?,
        receipt_handle: p.required_string("ReceiptHandle")?.to_owned(),
    })
}

fn parse_change_message_visibility(p: &Params) -> Result<Command> {
    p.require_all(&["QueueName", "ReceiptHandle", "VisibilityTimeout"])?;
    Ok(Command::ChangeMessageVisibility {
        queue: queue_name(p)?,
        receipt_handle: p.required_string("ReceiptHandle")?.to_owned(),
        visibility_timeout: p.required_integer("VisibilityTimeout", 0, MAX_VISIBILITY_TIMEOUT)?
            as u32,
    })
}
