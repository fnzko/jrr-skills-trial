//! Turns a request line into a typed command, applying the shape, required-parameter and
//! parameter-value checks of the specification in that order.

use std::fmt::Display;
use std::ops::RangeInclusive;

use serde_json::{Map, Value};

use crate::command::{Attributes, Command, SendMessage};
use crate::error::{CommandError, ErrorCode};
use crate::ids::{DedupKey, GroupId, QueueName};
use crate::time::Seconds;

const ATTRIBUTE_NAMES: [&str; 7] = [
    "VisibilityTimeout",
    "DelaySeconds",
    "RetentionSeconds",
    "Fifo",
    "ContentBasedDeduplication",
    "DeadLetterQueue",
    "MaxReceiveCount",
];

const VISIBILITY_TIMEOUTS: RangeInclusive<u32> = 0..=43_200;
const SEND_DELAYS: RangeInclusive<u32> = 0..=900;
const RETENTIONS: RangeInclusive<u32> = 60..=1_209_600;
const MAX_BODY_BYTES: usize = 1024;

/// Placeholder for the slots of `Params::required` until they are filled.
static ABSENT: Value = Value::Null;

/// Parses one line of input into a JSON value.
pub fn parse_line(line: &[u8]) -> Result<Value, CommandError> {
    serde_json::from_slice(line).map_err(|error| {
        CommandError::new(
            ErrorCode::InvalidRequest,
            format!("request is not valid UTF-8 JSON: {error}"),
        )
    })
}

/// Builds the command a request object names.
pub fn parse_command(request: &Value) -> Result<Command<'_>, CommandError> {
    let params = Params(request.as_object().ok_or_else(|| {
        CommandError::new(ErrorCode::InvalidRequest, "request is not a JSON object")
    })?);
    let name = params
        .get("Command")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            CommandError::new(
                ErrorCode::InvalidRequest,
                "request has no string member Command",
            )
        })?;
    match name {
        "CreateQueue" => parse_create_queue(&params),
        "SetQueueAttributes" => parse_set_queue_attributes(&params),
        "GetQueueAttributes" => Ok(Command::GetQueueAttributes {
            queue: params.queue_name()?,
        }),
        "ListQueues" => Ok(Command::ListQueues {
            prefix: params.string("Prefix")?,
        }),
        "DeleteQueue" => Ok(Command::DeleteQueue {
            queue: params.queue_name()?,
        }),
        "PurgeQueue" => Ok(Command::PurgeQueue {
            queue: params.queue_name()?,
        }),
        "Tick" => parse_tick(&params),
        "SendMessage" => parse_send_message(&params),
        "ReceiveMessage" => parse_receive_message(&params),
        "DeleteMessage" => parse_delete_message(&params),
        "ChangeMessageVisibility" => parse_change_message_visibility(&params),
        _ => Err(CommandError::new(
            ErrorCode::InvalidRequest,
            format!("unknown command {name:?}"),
        )),
    }
}

fn parse_create_queue<'a>(params: &Params<'a>) -> Result<Command<'a>, CommandError> {
    let [name] = params.required(["QueueName"])?;
    let name = as_string("QueueName", name)?;
    let name = QueueName::new(name).map_err(|_| {
        CommandError::new(
            ErrorCode::QueueNameInvalid,
            format!("{name:?} is not a valid queue name"),
        )
    })?;
    let attributes = match params.object("Attributes")? {
        Some(attributes) => parse_attributes(&attributes)?,
        None => Attributes::default(),
    };
    Ok(Command::CreateQueue { name, attributes })
}

fn parse_set_queue_attributes<'a>(params: &Params<'a>) -> Result<Command<'a>, CommandError> {
    let [queue, attributes] = params.required(["QueueName", "Attributes"])?;
    let queue = as_string("QueueName", queue)?;
    let attributes = parse_attributes(&as_object("Attributes", attributes)?)?;
    Ok(Command::SetQueueAttributes { queue, attributes })
}

fn parse_attributes<'a>(params: &Params<'a>) -> Result<Attributes<'a>, CommandError> {
    let unknown = params
        .present()
        .find(|(name, _)| !ATTRIBUTE_NAMES.contains(name));
    if let Some((name, _)) = unknown {
        return Err(CommandError::new(
            ErrorCode::InvalidParameterValue,
            format!("unknown attribute {name:?}"),
        ));
    }
    Ok(Attributes {
        visibility_timeout: params.seconds("VisibilityTimeout", VISIBILITY_TIMEOUTS)?,
        delay: params.seconds("DelaySeconds", SEND_DELAYS)?,
        retention: params.seconds("RetentionSeconds", RETENTIONS)?,
        fifo: params.boolean("Fifo")?,
        content_based_deduplication: params.boolean("ContentBasedDeduplication")?,
        dead_letter_queue: params.string("DeadLetterQueue")?,
        max_receive_count: params.integer("MaxReceiveCount", 1..=1000)?,
    })
}

fn parse_tick<'a>(params: &Params<'a>) -> Result<Command<'a>, CommandError> {
    let [seconds] = params.required(["Seconds"])?;
    let seconds = as_seconds("Seconds", seconds, 0..=1_209_600)?;
    Ok(Command::Tick { seconds })
}

fn parse_send_message<'a>(params: &Params<'a>) -> Result<Command<'a>, CommandError> {
    let [queue, body] = params.required(["QueueName", "Body"])?;
    let queue = as_string("QueueName", queue)?;
    let body = as_string("Body", body)?;
    if body.is_empty() || body.len() > MAX_BODY_BYTES {
        return Err(CommandError::invalid_value(
            "Body",
            format!("1 to {MAX_BODY_BYTES} bytes long"),
        ));
    }
    let group = params
        .string("GroupId")?
        .map(|id| GroupId::new(id).map_err(|_| CommandError::invalid_value("GroupId", TOKEN_RULE)))
        .transpose()?;
    let deduplication_id = params
        .string("DeduplicationId")?
        .map(|id| {
            DedupKey::explicit(id)
                .map_err(|_| CommandError::invalid_value("DeduplicationId", TOKEN_RULE))
        })
        .transpose()?;
    Ok(Command::SendMessage(SendMessage {
        queue,
        body,
        delay: params.seconds("DelaySeconds", SEND_DELAYS)?,
        group,
        deduplication_id,
    }))
}

const TOKEN_RULE: &str = "1 to 128 characters from A-Z a-z 0-9 _ -";

fn parse_receive_message<'a>(params: &Params<'a>) -> Result<Command<'a>, CommandError> {
    let queue = params.queue_name()?;
    Ok(Command::ReceiveMessage {
        queue,
        max_messages: params.integer("MaxMessages", 1..=10)?.unwrap_or(1),
        visibility_timeout: params.seconds("VisibilityTimeout", VISIBILITY_TIMEOUTS)?,
    })
}

fn parse_delete_message<'a>(params: &Params<'a>) -> Result<Command<'a>, CommandError> {
    let [queue, handle] = params.required(["QueueName", "ReceiptHandle"])?;
    Ok(Command::DeleteMessage {
        queue: as_string("QueueName", queue)?,
        receipt_handle: as_string("ReceiptHandle", handle)?,
    })
}

fn parse_change_message_visibility<'a>(params: &Params<'a>) -> Result<Command<'a>, CommandError> {
    let [queue, handle, timeout] =
        params.required(["QueueName", "ReceiptHandle", "VisibilityTimeout"])?;
    Ok(Command::ChangeMessageVisibility {
        queue: as_string("QueueName", queue)?,
        receipt_handle: as_string("ReceiptHandle", handle)?,
        visibility_timeout: as_seconds("VisibilityTimeout", timeout, VISIBILITY_TIMEOUTS)?,
    })
}

/// The members of a JSON object, where `null` counts as absent.
struct Params<'a>(&'a Map<String, Value>);

impl<'a> Params<'a> {
    fn get(&self, name: &str) -> Option<&'a Value> {
        self.0.get(name).filter(|value| !value.is_null())
    }

    fn present(&self) -> impl Iterator<Item = (&'a str, &'a Value)> + use<'a> {
        self.0
            .iter()
            .filter(|(_, value)| !value.is_null())
            .map(|(name, value)| (name.as_str(), value))
    }

    /// The values of parameters that must all be present, in the order asked for.
    fn required<const N: usize>(&self, names: [&str; N]) -> Result<[&'a Value; N], CommandError> {
        let mut values = [&ABSENT; N];
        for (slot, name) in values.iter_mut().zip(names) {
            *slot = self.get(name).ok_or_else(|| CommandError::missing(name))?;
        }
        Ok(values)
    }

    /// The `QueueName` of a command whose only other parameters are optional.
    fn queue_name(&self) -> Result<&'a str, CommandError> {
        let [name] = self.required(["QueueName"])?;
        as_string("QueueName", name)
    }

    fn string(&self, name: &str) -> Result<Option<&'a str>, CommandError> {
        self.get(name)
            .map(|value| as_string(name, value))
            .transpose()
    }

    fn boolean(&self, name: &str) -> Result<Option<bool>, CommandError> {
        self.get(name)
            .map(|value| {
                value
                    .as_bool()
                    .ok_or_else(|| CommandError::invalid_value(name, "a boolean"))
            })
            .transpose()
    }

    fn object(&self, name: &str) -> Result<Option<Params<'a>>, CommandError> {
        self.get(name)
            .map(|value| as_object(name, value))
            .transpose()
    }

    fn integer<T>(&self, name: &str, range: RangeInclusive<T>) -> Result<Option<T>, CommandError>
    where
        T: TryFrom<i64> + PartialOrd + Display,
    {
        self.get(name)
            .map(|value| as_integer(name, value, range))
            .transpose()
    }

    fn seconds(
        &self,
        name: &str,
        range: RangeInclusive<u32>,
    ) -> Result<Option<Seconds>, CommandError> {
        Ok(self.integer(name, range)?.map(Seconds))
    }
}

fn as_string<'a>(name: &str, value: &'a Value) -> Result<&'a str, CommandError> {
    value
        .as_str()
        .ok_or_else(|| CommandError::invalid_value(name, "a string"))
}

fn as_object<'a>(name: &str, value: &'a Value) -> Result<Params<'a>, CommandError> {
    value
        .as_object()
        .map(Params)
        .ok_or_else(|| CommandError::invalid_value(name, "an object"))
}

/// Accepts only numbers written as plain integers: `30`, but not `30.0`, `3e1` or `"30"`.
fn as_integer<T>(name: &str, value: &Value, range: RangeInclusive<T>) -> Result<T, CommandError>
where
    T: TryFrom<i64> + PartialOrd + Display,
{
    value
        .as_i64()
        .and_then(|number| T::try_from(number).ok())
        .filter(|number| range.contains(number))
        .ok_or_else(|| {
            CommandError::invalid_value(
                name,
                format!("an integer from {} to {}", range.start(), range.end()),
            )
        })
}

fn as_seconds(
    name: &str,
    value: &Value,
    range: RangeInclusive<u32>,
) -> Result<Seconds, CommandError> {
    as_integer(name, value, range).map(Seconds)
}
