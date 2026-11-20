//! The line protocol: one command line in, one JSON response line out.

use serde_json::{Map, Value, json};

use crate::error::Error;
use crate::queue::{Attributes, Counts, Delivery};
use crate::request::Request;
use crate::service::{Response, Service};

/// Runs one input line (without its trailing `\n`) and returns the response line (without one).
pub fn handle_line(service: &mut Service, line: &[u8]) -> String {
    let outcome = Request::from_line(line).and_then(|request| service.execute(request));
    let value = match outcome {
        Ok(response) => success(response),
        Err(error) => failure(&error),
    };
    value.to_string()
}

fn success(response: Response) -> Value {
    let mut object = Map::new();
    object.insert("ok".into(), Value::Bool(true));
    match response {
        Response::Done => {}
        Response::Now(now) => {
            object.insert("Now".into(), now.into());
        }
        Response::QueueNames(names) => {
            object.insert("QueueNames".into(), names.into());
        }
        Response::QueueAttributes { attributes, counts } => {
            object.insert("Attributes".into(), attributes_json(&attributes));
            object.insert("Counts".into(), counts_json(counts));
        }
        Response::Sent {
            message_id,
            sequence_number,
        } => {
            object.insert("MessageId".into(), message_id_json(message_id));
            if let Some(sequence_number) = sequence_number {
                object.insert("SequenceNumber".into(), sequence_number.into());
            }
        }
        Response::Messages(deliveries) => {
            let messages = deliveries.iter().map(delivery_json).collect();
            object.insert("Messages".into(), Value::Array(messages));
        }
    }
    Value::Object(object)
}

fn failure(error: &Error) -> Value {
    json!({
        "ok": false,
        "error": { "code": error.code.as_str(), "message": error.message },
    })
}

fn message_id_json(id: u64) -> Value {
    format!("m-{id}").into()
}

fn attributes_json(attributes: &Attributes) -> Value {
    let mut object = json!({
        "VisibilityTimeout": attributes.visibility_timeout,
        "DelaySeconds": attributes.delay_seconds,
        "RetentionSeconds": attributes.retention_seconds,
        "Fifo": attributes.fifo,
        "ContentBasedDeduplication": attributes.content_based_deduplication,
    });
    if let Some(config) = &attributes.dead_letter {
        object["DeadLetterQueue"] = config.queue_name.clone().into();
        object["MaxReceiveCount"] = config.max_receive_count.into();
    }
    object
}

fn counts_json(counts: Counts) -> Value {
    json!({
        "Visible": counts.visible,
        "InFlight": counts.in_flight,
        "Delayed": counts.delayed,
    })
}

fn delivery_json(delivery: &Delivery) -> Value {
    let mut object = json!({
        "MessageId": message_id_json(delivery.message_id),
        "ReceiptHandle": format!("r-{}", delivery.receipt),
        "Body": delivery.body,
        "ReceiveCount": delivery.receive_count,
        "SentAt": delivery.sent_at,
        "FirstReceivedAt": delivery.first_received_at,
    });
    if let Some(group_id) = &delivery.group_id {
        object["GroupId"] = group_id.clone().into();
    }
    if let Some(sequence_number) = delivery.sequence_number {
        object["SequenceNumber"] = sequence_number.into();
    }
    object
}
