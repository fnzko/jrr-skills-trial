#![allow(dead_code)] // each test binary uses a different subset

use miniq::broker::Broker;
use serde_json::{Value, json};

/// A broker driven with JSON commands.
pub struct Session(Broker);

impl Session {
    pub fn new() -> Self {
        Self(Broker::new())
    }

    pub fn run(&mut self, command: Value) -> Value {
        self.raw(&command.to_string())
    }

    pub fn raw(&mut self, line: &str) -> Value {
        self.0.handle_line(line.as_bytes())
    }

    /// Runs a command that must succeed and returns its response.
    pub fn ok(&mut self, command: Value) -> Value {
        let response = self.run(command.clone());
        assert_eq!(response["ok"], json!(true), "{command} failed: {response}");
        response
    }

    pub fn create(&mut self, name: &str, attributes: Value) {
        self.ok(json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attributes}));
    }

    pub fn tick(&mut self, seconds: u32) {
        self.ok(json!({"Command": "Tick", "Seconds": seconds}));
    }

    pub fn send(&mut self, queue: &str, body: &str) -> Value {
        self.ok(json!({"Command": "SendMessage", "QueueName": queue, "Body": body}))
    }

    pub fn receive(&mut self, queue: &str) -> Value {
        self.ok(json!({"Command": "ReceiveMessage", "QueueName": queue}))
    }

    pub fn counts(&mut self, queue: &str) -> Value {
        self.ok(json!({"Command": "GetQueueAttributes", "QueueName": queue}))["Counts"].clone()
    }
}

pub fn success() -> Value {
    json!({"ok": true})
}

/// The error code of a failure response; panics on a success.
pub fn error_code(response: &Value) -> &str {
    assert_eq!(
        response["ok"],
        json!(false),
        "expected a failure: {response}"
    );
    response["error"]["code"]
        .as_str()
        .expect("error code is a string")
}

/// The bodies of the messages in a ReceiveMessage response, in order.
pub fn bodies(response: &Value) -> Vec<&str> {
    response["Messages"]
        .as_array()
        .expect("Messages is an array")
        .iter()
        .map(|m| m["Body"].as_str().expect("Body is a string"))
        .collect()
}
