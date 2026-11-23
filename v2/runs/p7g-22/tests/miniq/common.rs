use std::io::Cursor;

use serde_json::{Value, json};

/// Feeds `lines` to a fresh miniq and returns one parsed response per line.
pub fn run(lines: &[Value]) -> Vec<Value> {
    let input: String = lines.iter().map(|line| format!("{line}\n")).collect();
    run_raw(input.as_bytes())
}

pub fn run_raw(input: &[u8]) -> Vec<Value> {
    let mut output = Vec::new();
    miniq::session::run(Cursor::new(input), &mut output).expect("in-memory io cannot fail");
    String::from_utf8(output)
        .expect("responses are UTF-8")
        .lines()
        .map(|line| serde_json::from_str(line).expect("each response is one JSON object"))
        .collect()
}

pub fn ok() -> Value {
    json!({"ok": true})
}

/// The `code` of a failure response; panics on success so a test cannot pass
/// by accident.
pub fn code(response: &Value) -> &str {
    assert_eq!(
        response["ok"],
        json!(false),
        "expected a failure: {response}"
    );
    response["error"]["code"]
        .as_str()
        .expect("error has a code")
}

pub fn create(name: &str) -> Value {
    json!({"Command": "CreateQueue", "QueueName": name})
}

pub fn create_with(name: &str, attributes: Value) -> Value {
    json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attributes})
}

pub fn send(queue: &str, body: &str) -> Value {
    json!({"Command": "SendMessage", "QueueName": queue, "Body": body})
}

pub fn send_fifo(queue: &str, body: &str, group: &str, dedup: &str) -> Value {
    json!({"Command": "SendMessage", "QueueName": queue, "Body": body,
           "GroupId": group, "DeduplicationId": dedup})
}

pub fn receive(queue: &str) -> Value {
    json!({"Command": "ReceiveMessage", "QueueName": queue})
}

pub fn receive_up_to(queue: &str, max: u64) -> Value {
    json!({"Command": "ReceiveMessage", "QueueName": queue, "MaxMessages": max})
}

pub fn tick(seconds: u64) -> Value {
    json!({"Command": "Tick", "Seconds": seconds})
}

pub fn attributes(queue: &str) -> Value {
    json!({"Command": "GetQueueAttributes", "QueueName": queue})
}

pub fn set_attributes(queue: &str, attributes: Value) -> Value {
    json!({"Command": "SetQueueAttributes", "QueueName": queue, "Attributes": attributes})
}

pub fn delete(queue: &str, handle: &str) -> Value {
    json!({"Command": "DeleteMessage", "QueueName": queue, "ReceiptHandle": handle})
}

pub fn change_visibility(queue: &str, handle: &str, timeout: u64) -> Value {
    json!({"Command": "ChangeMessageVisibility", "QueueName": queue,
           "ReceiptHandle": handle, "VisibilityTimeout": timeout})
}

pub fn counts(response: &Value) -> Value {
    response["Counts"].clone()
}

/// The `(MessageId, Body)` pairs of a receive response, in delivery order.
pub fn delivered(response: &Value) -> Vec<(String, String)> {
    response["Messages"]
        .as_array()
        .expect("receive responses carry Messages")
        .iter()
        .map(|m| {
            (
                m["MessageId"].as_str().expect("MessageId").to_owned(),
                m["Body"].as_str().expect("Body").to_owned(),
            )
        })
        .collect()
}
