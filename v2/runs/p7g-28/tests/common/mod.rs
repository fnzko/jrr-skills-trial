#![allow(dead_code)] // each test crate uses a different subset of these helpers

//! Runs the `miniq` binary on a script of input lines.

use std::io::Write;
use std::process::{Command, Stdio};

use serde_json::{Value, json};

pub struct Output {
    pub responses: Vec<Value>,
    pub raw: String,
    pub status: i32,
}

pub fn run_bytes(input: &[u8]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_miniq"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn miniq");
    // Feed stdin from a thread: a large script would otherwise fill the stdout
    // pipe while we are still writing, and deadlock.
    let mut stdin = child.stdin.take().unwrap();
    let input = input.to_vec();
    let writer = std::thread::spawn(move || stdin.write_all(&input).unwrap());
    let out = child.wait_with_output().unwrap();
    writer.join().unwrap();
    let raw = String::from_utf8(out.stdout).unwrap();
    let responses = raw
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    Output {
        responses,
        raw,
        status: out.status.code().unwrap(),
    }
}

pub fn run(lines: &[&str]) -> Output {
    let mut input = lines.join("\n");
    input.push('\n');
    run_bytes(input.as_bytes())
}

pub fn run_cmds(cmds: Vec<Value>) -> Vec<Value> {
    let lines: Vec<String> = cmds.iter().map(Value::to_string).collect();
    let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
    run(&refs).responses
}

/// The error code of a failed response; panics on success.
pub fn code(r: &Value) -> &str {
    assert_eq!(r["ok"], false, "{r}");
    r["error"]["code"].as_str().unwrap()
}

/// `"ok"` or the error code of each response.
pub fn codes(responses: &[Value]) -> Vec<&str> {
    responses
        .iter()
        .map(|r| r["error"]["code"].as_str().unwrap_or("ok"))
        .collect()
}

pub fn create(name: &str, attrs: Value) -> Value {
    json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attrs})
}

pub fn send(queue: &str, body: &str) -> Value {
    json!({"Command": "SendMessage", "QueueName": queue, "Body": body})
}

pub fn tick(seconds: i64) -> Value {
    json!({"Command": "Tick", "Seconds": seconds})
}

pub fn counts(queue: &str) -> Value {
    json!({"Command": "GetQueueAttributes", "QueueName": queue})
}
