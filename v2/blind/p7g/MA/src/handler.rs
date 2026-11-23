//! Turns one input line into one response object.

use serde_json::{Map, Value, json};

use crate::attributes::AttributeSet;
use crate::broker::{Broker, Delivery, SendRequest};
use crate::error::{Error, ErrorCode, Result};
use crate::params::Params;
use crate::queue::{QueueName, is_name_byte};

const MAX_BODY_BYTES: usize = 1024;
const MAX_ID_CHARS: usize = 128;

#[derive(Debug, Clone, Copy)]
enum Command {
    CreateQueue,
    SetQueueAttributes,
    GetQueueAttributes,
    ListQueues,
    DeleteQueue,
    PurgeQueue,
    Tick,
    SendMessage,
    ReceiveMessage,
    DeleteMessage,
    ChangeMessageVisibility,
}

impl Command {
    fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "CreateQueue" => Self::CreateQueue,
            "SetQueueAttributes" => Self::SetQueueAttributes,
            "GetQueueAttributes" => Self::GetQueueAttributes,
            "ListQueues" => Self::ListQueues,
            "DeleteQueue" => Self::DeleteQueue,
            "PurgeQueue" => Self::PurgeQueue,
            "Tick" => Self::Tick,
            "SendMessage" => Self::SendMessage,
            "ReceiveMessage" => Self::ReceiveMessage,
            "DeleteMessage" => Self::DeleteMessage,
            "ChangeMessageVisibility" => Self::ChangeMessageVisibility,
            _ => return None,
        })
    }
}

/// Runs the command on `line` (without its newline) and builds the response.
pub fn respond(broker: &mut Broker, line: &[u8]) -> Value {
    match execute(broker, line) {
        Ok(mut members) => {
            members.insert("ok".to_owned(), Value::Bool(true));
            Value::Object(members)
        }
        Err(error) => json!({
            "ok": false,
            "error": {"code": error.code().as_str(), "message": error.message()},
        }),
    }
}

fn invalid_request(message: impl Into<String>) -> Error {
    Error::new(ErrorCode::InvalidRequest, message)
}

fn execute(broker: &mut Broker, line: &[u8]) -> Result<Map<String, Value>> {
    let text = std::str::from_utf8(line)
        .map_err(|e| invalid_request(format!("the line is not valid UTF-8: {e}")))?;
    let Value::Object(members) = serde_json::from_str(text)
        .map_err(|e| invalid_request(format!("the line is not valid JSON: {e}")))?
    else {
        return Err(invalid_request("the line is not a JSON object"));
    };
    let command = members
        .get("Command")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid_request("Command must be a string"))?;
    let command = Command::parse(command)
        .ok_or_else(|| invalid_request(format!("unknown command {command:?}")))?;

    let params = Params::new(&members);
    match command {
        Command::CreateQueue => create_queue(broker, &params),
        Command::SetQueueAttributes => set_queue_attributes(broker, &params),
        Command::GetQueueAttributes => get_queue_attributes(broker, &params),
        Command::ListQueues => list_queues(broker, &params),
        Command::DeleteQueue => queue_only(&params, |name| broker.delete_queue(name)),
        Command::PurgeQueue => queue_only(&params, |name| broker.purge_queue(name)),
        Command::Tick => tick(broker, &params),
        Command::SendMessage => send_message(broker, &params),
        Command::ReceiveMessage => receive_message(broker, &params),
        Command::DeleteMessage => delete_message(broker, &params),
        Command::ChangeMessageVisibility => change_message_visibility(broker, &params),
    }
}

fn members(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map,
        _ => Map::new(),
    }
}

fn create_queue(broker: &mut Broker, params: &Params<'_>) -> Result<Map<String, Value>> {
    params.require(&["QueueName"])?;
    let name = params.required_string("QueueName")?;
    let name = QueueName::parse(name).ok_or_else(|| {
        Error::new(
            ErrorCode::QueueNameInvalid,
            format!("{name:?} is not a valid queue name"),
        )
    })?;
    let attributes = params
        .object("Attributes")?
        .map(AttributeSet::parse)
        .transpose()?
        .unwrap_or_default();
    broker.create_queue(name, &attributes)?;
    Ok(Map::new())
}

fn set_queue_attributes(broker: &mut Broker, params: &Params<'_>) -> Result<Map<String, Value>> {
    params.require(&["QueueName", "Attributes"])?;
    let name = params.required_string("QueueName")?;
    let attributes = params
        .object("Attributes")?
        .map(AttributeSet::parse)
        .transpose()?
        .unwrap_or_default();
    broker.set_queue_attributes(name, &attributes)?;
    Ok(Map::new())
}

fn get_queue_attributes(broker: &mut Broker, params: &Params<'_>) -> Result<Map<String, Value>> {
    params.require(&["QueueName"])?;
    let view = broker.queue_view(params.required_string("QueueName")?)?;
    let settings = &view.settings;
    let mut attributes = json!({
        "VisibilityTimeout": settings.visibility_timeout.get(),
        "DelaySeconds": settings.delay.get(),
        "RetentionSeconds": settings.retention.get(),
        "Fifo": settings.queue_type.is_fifo(),
        "ContentBasedDeduplication": settings.queue_type.content_based_dedup(),
    });
    if let (Some(dead_letter), Some(map)) = (&settings.dead_letter, attributes.as_object_mut()) {
        map.insert(
            "DeadLetterQueue".to_owned(),
            json!(dead_letter.queue.as_str()),
        );
        map.insert(
            "MaxReceiveCount".to_owned(),
            json!(dead_letter.max_receive_count),
        );
    }
    Ok(members(json!({
        "Attributes": attributes,
        "Counts": {
            "Visible": view.counts.visible,
            "InFlight": view.counts.in_flight,
            "Delayed": view.counts.delayed,
        },
    })))
}

fn list_queues(broker: &Broker, params: &Params<'_>) -> Result<Map<String, Value>> {
    let prefix = params.string("Prefix")?.unwrap_or("");
    Ok(members(json!({"QueueNames": broker.list_queues(prefix)})))
}

/// Commands whose only parameter is the queue name and that return nothing.
fn queue_only(
    params: &Params<'_>,
    run: impl FnOnce(&str) -> Result<()>,
) -> Result<Map<String, Value>> {
    params.require(&["QueueName"])?;
    run(params.required_string("QueueName")?)?;
    Ok(Map::new())
}

fn tick(broker: &mut Broker, params: &Params<'_>) -> Result<Map<String, Value>> {
    params.require(&["Seconds"])?;
    let seconds = params.required_seconds("Seconds", 0..=1_209_600)?;
    Ok(members(json!({"Now": broker.tick(seconds).get()})))
}

/// A `GroupId` or `DeduplicationId`: 1 to 128 name characters.
fn id_param<'a>(params: &Params<'a>, name: &str) -> Result<Option<&'a str>> {
    let value = params.string(name)?;
    match value {
        Some(id) if id.is_empty() || id.len() > MAX_ID_CHARS || !id.bytes().all(is_name_byte) => {
            Err(Error::new(
                ErrorCode::InvalidParameterValue,
                format!("{name} must be 1 to {MAX_ID_CHARS} characters of A-Z a-z 0-9 _ -"),
            ))
        }
        _ => Ok(value),
    }
}

fn send_message(broker: &mut Broker, params: &Params<'_>) -> Result<Map<String, Value>> {
    params.require(&["QueueName", "Body"])?;
    let name = params.required_string("QueueName")?;
    let body = params.required_string("Body")?;
    if body.is_empty() || body.len() > MAX_BODY_BYTES {
        return Err(Error::new(
            ErrorCode::InvalidParameterValue,
            format!("Body must be 1 to {MAX_BODY_BYTES} bytes"),
        ));
    }
    let request = SendRequest {
        body,
        delay: params.seconds("DelaySeconds", 0..=900)?,
        group: id_param(params, "GroupId")?,
        dedup_id: id_param(params, "DeduplicationId")?,
    };
    let accepted = broker.send_message(name, &request)?;
    let mut result = Map::new();
    result.insert("MessageId".to_owned(), json!(accepted.id.to_string()));
    if let Some(sequence) = accepted.sequence {
        result.insert("SequenceNumber".to_owned(), json!(sequence.get()));
    }
    Ok(result)
}

fn delivery_json(delivery: &Delivery) -> Value {
    let mut message = json!({
        "MessageId": delivery.id.to_string(),
        "ReceiptHandle": delivery.receipt.to_string(),
        "Body": delivery.body,
        "ReceiveCount": delivery.receive_count,
        "SentAt": delivery.sent_at.get(),
        "FirstReceivedAt": delivery.first_received_at.map(|t| t.get()),
    });
    if let (Some(fifo), Some(map)) = (&delivery.fifo, message.as_object_mut()) {
        map.insert("GroupId".to_owned(), json!(fifo.group));
        map.insert("SequenceNumber".to_owned(), json!(fifo.sequence.get()));
    }
    message
}

fn receive_message(broker: &mut Broker, params: &Params<'_>) -> Result<Map<String, Value>> {
    params.require(&["QueueName"])?;
    let name = params.required_string("QueueName")?;
    let max = params.integer("MaxMessages", 1..=10)?.unwrap_or(1);
    let visibility = params.seconds("VisibilityTimeout", 0..=43_200)?;
    // The range 1..=10 is checked above, so the conversion cannot fail.
    let max = usize::try_from(max).unwrap_or(1);
    let deliveries = broker.receive_messages(name, max, visibility)?;
    let messages: Vec<Value> = deliveries.iter().map(delivery_json).collect();
    Ok(members(json!({"Messages": messages})))
}

fn delete_message(broker: &mut Broker, params: &Params<'_>) -> Result<Map<String, Value>> {
    params.require(&["QueueName", "ReceiptHandle"])?;
    let name = params.required_string("QueueName")?;
    let handle = params.required_string("ReceiptHandle")?;
    broker.delete_message(name, handle)?;
    Ok(Map::new())
}

fn change_message_visibility(
    broker: &mut Broker,
    params: &Params<'_>,
) -> Result<Map<String, Value>> {
    params.require(&["QueueName", "ReceiptHandle", "VisibilityTimeout"])?;
    let name = params.required_string("QueueName")?;
    let handle = params.required_string("ReceiptHandle")?;
    let timeout = params.required_seconds("VisibilityTimeout", 0..=43_200)?;
    broker.change_message_visibility(name, handle, timeout)?;
    Ok(Map::new())
}
