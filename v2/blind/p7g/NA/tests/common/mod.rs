//! Test helpers: run commands against a fresh broker and inspect the replies.
#![allow(dead_code)]

use miniq::Broker;
use serde_json::Value;

pub struct Session {
    broker: Broker,
}

impl Session {
    pub fn new() -> Self {
        Self {
            broker: Broker::new(),
        }
    }

    /// Sends one raw line and returns the parsed response.
    pub fn raw(&mut self, line: &str) -> Value {
        let response = miniq::respond(&mut self.broker, line.as_bytes());
        serde_json::from_str(&response).expect("response is JSON")
    }

    /// Sends a command given as a JSON value.
    pub fn send(&mut self, command: Value) -> Value {
        self.raw(&command.to_string())
    }

    /// Sends a command and asserts that it succeeded.
    pub fn ok(&mut self, command: Value) -> Value {
        let response = self.send(command);
        assert_eq!(response["ok"], true, "expected success, got {response}");
        response
    }

    /// Sends a command and returns the error code it failed with.
    pub fn err(&mut self, command: Value) -> String {
        let response = self.send(command);
        assert_eq!(response["ok"], false, "expected failure, got {response}");
        response["error"]["code"].as_str().expect("code").to_owned()
    }

    pub fn tick(&mut self, seconds: u32) {
        self.ok(serde_json::json!({"Command": "Tick", "Seconds": seconds}));
    }

    pub fn create(&mut self, name: &str, attributes: Value) {
        self.ok(serde_json::json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attributes}));
    }

    pub fn push(&mut self, queue: &str, body: &str) -> String {
        let r = self
            .ok(serde_json::json!({"Command": "SendMessage", "QueueName": queue, "Body": body}));
        r["MessageId"].as_str().unwrap().to_owned()
    }

    pub fn receive(&mut self, queue: &str, max: u32) -> Vec<Value> {
        let r = self.ok(serde_json::json!({"Command": "ReceiveMessage", "QueueName": queue, "MaxMessages": max}));
        r["Messages"].as_array().unwrap().clone()
    }

    pub fn counts(&mut self, queue: &str) -> [u64; 3] {
        let r = self.ok(serde_json::json!({"Command": "GetQueueAttributes", "QueueName": queue}));
        let c = &r["Counts"];
        [
            c["Visible"].as_u64().unwrap(),
            c["InFlight"].as_u64().unwrap(),
            c["Delayed"].as_u64().unwrap(),
        ]
    }
}
