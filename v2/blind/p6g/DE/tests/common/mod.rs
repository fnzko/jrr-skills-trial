//! Drives the service through its line protocol.

#![allow(
    dead_code,
    reason = "each test binary uses a different subset of these helpers"
)]

use serde_json::Value;

/// Runs the lines through a fresh service and returns one response per line.
pub fn run(lines: &[&str]) -> Vec<Value> {
    let mut input = lines.join("\n");
    input.push('\n');
    run_raw(input.as_bytes())
}

pub fn run_raw(input: &[u8]) -> Vec<Value> {
    let mut output = Vec::new();
    miniq::session::serve(input, &mut output).unwrap();
    String::from_utf8(output)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

/// The code of a failure response; panics on a success.
#[track_caller]
pub fn error_code(response: &Value) -> &str {
    assert_eq!(response["ok"], false, "response {response}");
    response["error"]["code"].as_str().unwrap()
}

/// Runs commands given as JSON values.
pub fn run_values(commands: &[Value]) -> Vec<Value> {
    let lines: Vec<String> = commands.iter().map(Value::to_string).collect();
    let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
    run(&lines)
}

pub fn create(name: &str, attributes: Value) -> Value {
    serde_json::json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attributes})
}

pub fn send(queue: &str, body: &str) -> Value {
    serde_json::json!({"Command": "SendMessage", "QueueName": queue, "Body": body})
}

pub fn receive(queue: &str) -> Value {
    serde_json::json!({"Command": "ReceiveMessage", "QueueName": queue})
}

pub fn receive_many(queue: &str, max: u64) -> Value {
    serde_json::json!({"Command": "ReceiveMessage", "QueueName": queue, "MaxMessages": max})
}

pub fn tick(seconds: u64) -> Value {
    serde_json::json!({"Command": "Tick", "Seconds": seconds})
}

pub fn counts(queue: &str) -> Value {
    serde_json::json!({"Command": "GetQueueAttributes", "QueueName": queue})
}

pub fn delete(queue: &str, handle: &str) -> Value {
    serde_json::json!({"Command": "DeleteMessage", "QueueName": queue, "ReceiptHandle": handle})
}

pub fn change_visibility(queue: &str, handle: &str, timeout: u64) -> Value {
    serde_json::json!({
        "Command": "ChangeMessageVisibility",
        "QueueName": queue,
        "ReceiptHandle": handle,
        "VisibilityTimeout": timeout
    })
}

/// The bodies of a `ReceiveMessage` response, in delivery order.
#[track_caller]
pub fn bodies(response: &Value) -> Vec<&str> {
    assert_eq!(response["ok"], true, "response {response}");
    response["Messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["Body"].as_str().unwrap())
        .collect()
}

/// `[visible, in_flight, delayed]` of a `GetQueueAttributes` response.
#[track_caller]
pub fn state_counts(response: &Value) -> [u64; 3] {
    let counts = &response["Counts"];
    ["Visible", "InFlight", "Delayed"].map(|k| counts[k].as_u64().unwrap())
}
