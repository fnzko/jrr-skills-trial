#![allow(dead_code)]

use miniq::Service;
use serde_json::{Value, json};

/// A service driven with JSON values instead of text lines.
pub struct Session(Service);

impl Session {
    pub fn new() -> Self {
        Self(Service::new())
    }

    /// Runs one command and returns the whole response.
    pub fn run(&mut self, command: Value) -> Value {
        self.0.handle_line(command.to_string().as_bytes())
    }

    /// Runs a command that must succeed.
    pub fn ok(&mut self, command: Value) -> Value {
        let response = self.run(command.clone());
        assert_eq!(response["ok"], true, "{command} -> {response}");
        response
    }

    /// Runs a command that must fail and returns the error code.
    pub fn err(&mut self, command: Value) -> String {
        let response = self.run(command.clone());
        assert_eq!(response["ok"], false, "{command} -> {response}");
        response["error"]["code"].as_str().unwrap().to_owned()
    }

    pub fn create(&mut self, name: &str, attributes: Value) {
        self.ok(json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attributes}));
    }

    pub fn tick(&mut self, seconds: u64) {
        self.ok(json!({"Command": "Tick", "Seconds": seconds}));
    }

    pub fn counts(&mut self, queue: &str) -> Value {
        self.ok(json!({"Command": "GetQueueAttributes", "QueueName": queue}))["Counts"].clone()
    }
}
