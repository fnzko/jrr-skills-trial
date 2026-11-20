//! Runs the real binary on a script and parses its answers.

// Each test file uses a different subset of these helpers.
#![allow(dead_code)]

use std::io::Write;
use std::process::{Command, Stdio};

use serde_json::{Map, Value, json};

/// `{"Command": name, ...params}`.
pub fn cmd(name: &str, params: Value) -> Value {
    let Value::Object(mut members) = params else {
        panic!("parameters must be an object");
    };
    members.insert("Command".to_owned(), json!(name));
    Value::Object(Map::from_iter(members))
}

pub fn run_raw(input: &[u8]) -> Vec<Value> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_miniq"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    // Feed stdin from another thread: a long script fills the stdout pipe
    // before it is all written, and nobody reads stdout until the writer ends.
    let input = input.to_vec();
    let writer = std::thread::spawn(move || stdin.write_all(&input).unwrap());
    let output = child.wait_with_output().unwrap();
    writer.join().unwrap();
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.is_empty() || text.ends_with('\n'));
    text.lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

pub fn run(commands: &[Value]) -> Vec<Value> {
    let mut input = String::new();
    for command in commands {
        input.push_str(&command.to_string());
        input.push('\n');
    }
    let responses = run_raw(input.as_bytes());
    assert_eq!(responses.len(), commands.len());
    responses
}

pub fn ok() -> Value {
    json!({"ok": true})
}

/// The `code` of a failure response, or a panic if the response succeeded.
pub fn code(response: &Value) -> &str {
    assert_eq!(response["ok"], json!(false), "{response}");
    assert!(response["error"]["message"].is_string());
    response["error"]["code"].as_str().unwrap()
}

pub fn create(name: &str) -> Value {
    cmd("CreateQueue", json!({"QueueName": name}))
}

pub fn create_with(name: &str, attributes: Value) -> Value {
    cmd(
        "CreateQueue",
        json!({"QueueName": name, "Attributes": attributes}),
    )
}

pub fn send(queue: &str, body: &str) -> Value {
    cmd("SendMessage", json!({"QueueName": queue, "Body": body}))
}

pub fn receive(queue: &str) -> Value {
    cmd("ReceiveMessage", json!({"QueueName": queue}))
}

pub fn tick(seconds: u64) -> Value {
    cmd("Tick", json!({"Seconds": seconds}))
}

pub fn counts(queue: &str) -> Value {
    cmd("GetQueueAttributes", json!({"QueueName": queue}))
}

/// The bodies of a `ReceiveMessage` response, in order.
pub fn bodies(response: &Value) -> Vec<&str> {
    response["Messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["Body"].as_str().unwrap())
        .collect()
}

/// `"ok"` for a success, otherwise the error code.
pub fn outcome(response: &Value) -> &str {
    if response["ok"] == json!(true) {
        "ok"
    } else {
        code(response)
    }
}
