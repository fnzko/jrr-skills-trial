//! Test harness: drives miniq through its public line protocol.
#![allow(dead_code)]

use miniq::broker::Broker;
use miniq::protocol::respond;
use serde_json::{Value, json};

/// A running miniq instance that tests send commands to.
pub struct Session(Broker);

impl Session {
    pub fn new() -> Self {
        Self(Broker::new())
    }

    /// Sends one raw line and returns the parsed response.
    pub fn raw(&mut self, line: &str) -> Value {
        let response = respond(&mut self.0, line.as_bytes());
        serde_json::from_str(&response).expect("response is JSON")
    }

    /// Sends a command given as a JSON value.
    pub fn send(&mut self, command: Value) -> Value {
        self.raw(&command.to_string())
    }

    /// Sends a command and asserts that it succeeded.
    pub fn ok(&mut self, command: Value) -> Value {
        let response = self.send(command);
        assert_eq!(response["ok"], true, "unexpected failure: {response}");
        response
    }

    /// Sends a command and returns the error code it failed with.
    pub fn err(&mut self, command: Value) -> String {
        let response = self.send(command);
        assert_eq!(response["ok"], false, "unexpected success: {response}");
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
