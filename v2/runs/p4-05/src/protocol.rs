//! Line framing and response rendering (SPEC.md section 1).

use std::io::{self, BufRead, Write};

use serde_json::{json, Map, Value};

use crate::attributes::Attributes;
use crate::broker::{Broker, Reply};
use crate::command;
use crate::queue::Delivery;
use crate::{Error, ErrorCode};

/// Reads commands line by line from `input` and writes one response line per
/// command to `output`, flushing each response before reading the next line.
///
/// Returns `Ok(())` at end of input. Errors are only I/O failures; a bad
/// command gets an `ok: false` response instead.
pub fn serve<R: BufRead, W: Write>(mut input: R, mut output: W) -> io::Result<()> {
    let mut broker = Broker::new();
    let mut line = Vec::new();
    loop {
        line.clear();
        if input.read_until(b'\n', &mut line)? == 0 {
            return Ok(());
        }
        if line.last() == Some(&b'\n') {
            line.pop();
        }
        let response = render(handle_line(&mut broker, &line));
        serde_json::to_writer(&mut output, &response)?;
        output.write_all(b"\n")?;
        output.flush()?;
    }
}

fn handle_line(broker: &mut Broker, line: &[u8]) -> Result<Reply, Error> {
    let text = std::str::from_utf8(line)
        .map_err(|_| Error::new(ErrorCode::InvalidRequest, "line is not valid UTF-8"))?;
    let value: Value = serde_json::from_str(text).map_err(|err| {
        Error::new(
            ErrorCode::InvalidRequest,
            format!("line is not JSON: {err}"),
        )
    })?;
    let Value::Object(object) = value else {
        return Err(Error::new(
            ErrorCode::InvalidRequest,
            "line is not a JSON object",
        ));
    };
    broker.execute(command::parse(&object)?)
}

fn render(outcome: Result<Reply, Error>) -> Value {
    match outcome {
        Ok(reply) => {
            let mut members = Map::new();
            members.insert("ok".to_owned(), Value::Bool(true));
            members.extend(reply_members(reply));
            Value::Object(members)
        }
        Err(error) => json!({
            "ok": false,
            "error": {"code": error.code.as_str(), "message": error.message},
        }),
    }
}

fn reply_members(reply: Reply) -> Map<String, Value> {
    let mut members = Map::new();
    match reply {
        Reply::Empty => {}
        Reply::QueueAttributes { attributes, counts } => {
            members.insert("Attributes".to_owned(), attributes_json(&attributes));
            members.insert(
                "Counts".to_owned(),
                json!({
                    "Visible": counts.visible,
                    "InFlight": counts.in_flight,
                    "Delayed": counts.delayed,
                }),
            );
        }
        Reply::QueueNames(names) => {
            members.insert("QueueNames".to_owned(), json!(names));
        }
        Reply::Now(now) => {
            members.insert("Now".to_owned(), json!(now));
        }
        Reply::Sent {
            message_id,
            sequence_number,
        } => {
            members.insert("MessageId".to_owned(), json!(format!("m-{message_id}")));
            if let Some(sequence_number) = sequence_number {
                members.insert("SequenceNumber".to_owned(), json!(sequence_number));
            }
        }
        Reply::Received(deliveries) => {
            let messages: Vec<Value> = deliveries.iter().map(delivery_json).collect();
            members.insert("Messages".to_owned(), Value::Array(messages));
        }
    }
    members
}

fn delivery_json(delivery: &Delivery) -> Value {
    let mut message = Map::new();
    message.insert(
        "MessageId".to_owned(),
        json!(format!("m-{}", delivery.message_id)),
    );
    message.insert(
        "ReceiptHandle".to_owned(),
        json!(format!("r-{}", delivery.receipt)),
    );
    message.insert("Body".to_owned(), json!(delivery.body));
    message.insert("ReceiveCount".to_owned(), json!(delivery.receive_count));
    message.insert("SentAt".to_owned(), json!(delivery.sent_at));
    message.insert(
        "FirstReceivedAt".to_owned(),
        json!(delivery.first_received_at),
    );
    if let Some(group_id) = &delivery.group_id {
        message.insert("GroupId".to_owned(), json!(group_id));
    }
    if let Some(sequence_number) = delivery.sequence_number {
        message.insert("SequenceNumber".to_owned(), json!(sequence_number));
    }
    Value::Object(message)
}

fn attributes_json(attributes: &Attributes) -> Value {
    let mut members = Map::new();
    members.insert(
        "VisibilityTimeout".to_owned(),
        json!(attributes.visibility_timeout),
    );
    members.insert("DelaySeconds".to_owned(), json!(attributes.delay_seconds));
    members.insert(
        "RetentionSeconds".to_owned(),
        json!(attributes.retention_seconds),
    );
    members.insert("Fifo".to_owned(), json!(attributes.fifo));
    members.insert(
        "ContentBasedDeduplication".to_owned(),
        json!(attributes.content_based_deduplication),
    );
    if let Some(dead_letter) = &attributes.dead_letter {
        members.insert("DeadLetterQueue".to_owned(), json!(dead_letter.queue));
        members.insert(
            "MaxReceiveCount".to_owned(),
            json!(dead_letter.max_receive_count),
        );
    }
    Value::Object(members)
}
