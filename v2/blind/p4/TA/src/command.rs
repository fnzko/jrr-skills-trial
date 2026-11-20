//! Turns a parsed command object into a typed `Command`.
//!
//! Covers steps 1 to 3 of SPEC.md section 2: the command name, the presence of
//! required parameters, then each parameter's type and range. Checks that need
//! queue state happen later, in the broker.

use std::ops::RangeInclusive;

use serde_json::{Map, Value};

use crate::attributes::AttributeChanges;
use crate::{Error, ErrorCode};

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    CreateQueue {
        name: String,
        attributes: AttributeChanges,
    },
    SetQueueAttributes {
        name: String,
        changes: AttributeChanges,
    },
    GetQueueAttributes {
        name: String,
    },
    ListQueues {
        prefix: Option<String>,
    },
    DeleteQueue {
        name: String,
    },
    PurgeQueue {
        name: String,
    },
    Tick {
        seconds: u32,
    },
    SendMessage(SendRequest),
    ReceiveMessage(ReceiveRequest),
    DeleteMessage {
        queue: String,
        receipt: String,
    },
    ChangeMessageVisibility {
        queue: String,
        receipt: String,
        visibility_timeout: u32,
    },
}

#[derive(Debug, PartialEq, Eq)]
pub struct SendRequest {
    pub queue: String,
    pub body: String,
    pub delay_seconds: Option<u32>,
    pub group_id: Option<String>,
    pub deduplication_id: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ReceiveRequest {
    pub queue: String,
    pub max_messages: u32,
    pub visibility_timeout: Option<u32>,
}

pub fn parse(object: &Map<String, Value>) -> Result<Command, Error> {
    let Some(Value::String(command)) = object.get("Command") else {
        return Err(invalid_request("missing string Command member"));
    };
    match command.as_str() {
        "CreateQueue" => create_queue(object),
        "SetQueueAttributes" => set_queue_attributes(object),
        "GetQueueAttributes" => Ok(Command::GetQueueAttributes {
            name: queue_name(object)?,
        }),
        "ListQueues" => list_queues(object),
        "DeleteQueue" => Ok(Command::DeleteQueue {
            name: queue_name(object)?,
        }),
        "PurgeQueue" => Ok(Command::PurgeQueue {
            name: queue_name(object)?,
        }),
        "Tick" => tick(object),
        "SendMessage" => send_message(object),
        "ReceiveMessage" => receive_message(object),
        "DeleteMessage" => {
            let (queue, receipt) = queue_and_receipt(object)?;
            Ok(Command::DeleteMessage { queue, receipt })
        }
        "ChangeMessageVisibility" => change_message_visibility(object),
        other => Err(invalid_request(format!("unknown command {other}"))),
    }
}

fn change_message_visibility(object: &Map<String, Value>) -> Result<Command, Error> {
    let queue = required(object, "QueueName")?;
    let receipt = required(object, "ReceiptHandle")?;
    let visibility = required(object, "VisibilityTimeout")?;
    Ok(Command::ChangeMessageVisibility {
        queue: as_string("QueueName", queue)?.to_owned(),
        receipt: as_string("ReceiptHandle", receipt)?.to_owned(),
        visibility_timeout: integer("VisibilityTimeout", visibility, 0..=43_200)?,
    })
}

/// The QueueName and ReceiptHandle of a message command.
fn queue_and_receipt(object: &Map<String, Value>) -> Result<(String, String), Error> {
    let queue = required(object, "QueueName")?;
    let receipt = required(object, "ReceiptHandle")?;
    Ok((
        as_string("QueueName", queue)?.to_owned(),
        as_string("ReceiptHandle", receipt)?.to_owned(),
    ))
}

fn send_message(object: &Map<String, Value>) -> Result<Command, Error> {
    let queue = required(object, "QueueName")?;
    let body = required(object, "Body")?;
    let body = as_string("Body", body)?;
    if !(1..=1024).contains(&body.len()) {
        return Err(invalid_value("Body", "must be 1 to 1024 bytes"));
    }
    let delay_seconds = optional(object, "DelaySeconds")
        .map(|value| integer("DelaySeconds", value, 0..=900))
        .transpose()?;
    let group_id = optional(object, "GroupId")
        .map(|value| identifier("GroupId", value))
        .transpose()?;
    let deduplication_id = optional(object, "DeduplicationId")
        .map(|value| identifier("DeduplicationId", value))
        .transpose()?;
    Ok(Command::SendMessage(SendRequest {
        queue: as_string("QueueName", queue)?.to_owned(),
        body: body.to_owned(),
        delay_seconds,
        group_id,
        deduplication_id,
    }))
}

fn receive_message(object: &Map<String, Value>) -> Result<Command, Error> {
    let queue = queue_name(object)?;
    let max_messages = optional(object, "MaxMessages")
        .map(|value| integer("MaxMessages", value, 1..=10))
        .transpose()?
        .unwrap_or(1);
    let visibility_timeout = optional(object, "VisibilityTimeout")
        .map(|value| integer("VisibilityTimeout", value, 0..=43_200))
        .transpose()?;
    Ok(Command::ReceiveMessage(ReceiveRequest {
        queue,
        max_messages,
        visibility_timeout,
    }))
}

fn create_queue(object: &Map<String, Value>) -> Result<Command, Error> {
    // QueueName is checked before any other parameter (SPEC.md section 2, step 3).
    let name = as_string("QueueName", required(object, "QueueName")?)?;
    if !is_valid_queue_name(name) {
        return Err(Error::new(
            ErrorCode::QueueNameInvalid,
            "a queue name is 1 to 80 characters of A-Z a-z 0-9 _ -, or 1 to 75 of those followed by .fifo",
        ));
    }
    let attributes = match optional(object, "Attributes") {
        Some(value) => attribute_changes(value)?,
        None => AttributeChanges::default(),
    };
    Ok(Command::CreateQueue {
        name: name.to_owned(),
        attributes,
    })
}

fn set_queue_attributes(object: &Map<String, Value>) -> Result<Command, Error> {
    let name = required(object, "QueueName")?;
    let attributes = required(object, "Attributes")?;
    Ok(Command::SetQueueAttributes {
        name: as_string("QueueName", name)?.to_owned(),
        changes: attribute_changes(attributes)?,
    })
}

fn list_queues(object: &Map<String, Value>) -> Result<Command, Error> {
    let prefix = optional(object, "Prefix")
        .map(|value| as_string("Prefix", value).map(str::to_owned))
        .transpose()?;
    Ok(Command::ListQueues { prefix })
}

fn tick(object: &Map<String, Value>) -> Result<Command, Error> {
    let seconds = integer("Seconds", required(object, "Seconds")?, 0..=1_209_600)?;
    Ok(Command::Tick { seconds })
}

/// Parses an `Attributes` object. A member that is `null` counts as absent.
fn attribute_changes(value: &Value) -> Result<AttributeChanges, Error> {
    let Value::Object(members) = value else {
        return Err(invalid_value("Attributes", "must be an object"));
    };
    let mut changes = AttributeChanges::default();
    for (name, value) in members.iter().filter(|(_, value)| !value.is_null()) {
        match name.as_str() {
            "VisibilityTimeout" => {
                changes.visibility_timeout = Some(integer(name, value, 0..=43_200)?);
            }
            "DelaySeconds" => changes.delay_seconds = Some(integer(name, value, 0..=900)?),
            "RetentionSeconds" => {
                changes.retention_seconds = Some(integer(name, value, 60..=1_209_600)?);
            }
            "Fifo" => changes.fifo = Some(boolean(name, value)?),
            "ContentBasedDeduplication" => {
                changes.content_based_deduplication = Some(boolean(name, value)?);
            }
            "DeadLetterQueue" => {
                changes.dead_letter_queue = Some(as_string(name, value)?.to_owned());
            }
            "MaxReceiveCount" => changes.max_receive_count = Some(integer(name, value, 1..=1_000)?),
            _ => return Err(invalid_value(name, "is not a queue attribute")),
        }
    }
    Ok(changes)
}

fn queue_name(object: &Map<String, Value>) -> Result<String, Error> {
    Ok(as_string("QueueName", required(object, "QueueName")?)?.to_owned())
}

/// A required parameter: absent and `null` both count as missing.
fn required<'a>(object: &'a Map<String, Value>, name: &str) -> Result<&'a Value, Error> {
    optional(object, name)
        .ok_or_else(|| Error::new(ErrorCode::MissingParameter, format!("{name} is required")))
}

/// An optional parameter: absent and `null` both count as not given.
fn optional<'a>(object: &'a Map<String, Value>, name: &str) -> Option<&'a Value> {
    object.get(name).filter(|value| !value.is_null())
}

fn as_string<'a>(name: &str, value: &'a Value) -> Result<&'a str, Error> {
    value
        .as_str()
        .ok_or_else(|| invalid_value(name, "must be a string"))
}

/// A GroupId or DeduplicationId: 1 to 128 characters of `A-Z a-z 0-9 _ -`.
fn identifier(name: &str, value: &Value) -> Result<String, Error> {
    let text = as_string(name, value)?;
    let valid = (1..=128).contains(&text.len())
        && text
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-');
    if !valid {
        return Err(invalid_value(
            name,
            "must be 1 to 128 characters of A-Z a-z 0-9 _ -",
        ));
    }
    Ok(text.to_owned())
}

fn boolean(name: &str, value: &Value) -> Result<bool, Error> {
    value
        .as_bool()
        .ok_or_else(|| invalid_value(name, "must be true or false"))
}

/// A JSON integer in `range`. Numbers written with a fraction or exponent,
/// such as `30.0` or `3e1`, are not integers and are rejected.
fn integer(name: &str, value: &Value, range: RangeInclusive<u32>) -> Result<u32, Error> {
    value
        .as_u64()
        .and_then(|number| u32::try_from(number).ok())
        .filter(|number| range.contains(number))
        .ok_or_else(|| {
            invalid_value(
                name,
                &format!(
                    "must be an integer from {} to {}",
                    range.start(),
                    range.end()
                ),
            )
        })
}

/// A queue name: 1 to 80 characters of `A-Z a-z 0-9 _ -`, or 1 to 75 of those
/// followed by `.fifo`.
fn is_valid_queue_name(name: &str) -> bool {
    let (base, max_len) = match name.strip_suffix(".fifo") {
        Some(base) => (base, 75),
        None => (name, 80),
    };
    (1..=max_len).contains(&base.len())
        && base
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

fn invalid_request(message: impl Into<String>) -> Error {
    Error::new(ErrorCode::InvalidRequest, message)
}

fn invalid_value(name: &str, expectation: &str) -> Error {
    Error::new(
        ErrorCode::InvalidParameterValue,
        format!("{name} {expectation}"),
    )
}
