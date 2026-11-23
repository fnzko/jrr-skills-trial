//! Shared helpers: drive a `Broker` with JSON values and assert on responses.
#![allow(dead_code)]

use miniq::Broker;
use serde_json::{Value, json};

pub struct Session {
    broker: Broker,
}

impl Session {
    pub fn new() -> Self {
        Self {
            broker: Broker::new(),
        }
    }

    pub fn send(&mut self, command: Value) -> Value {
        self.broker.handle_line(command.to_string().as_bytes())
    }

    pub fn raw(&mut self, line: &str) -> Value {
        self.broker.handle_line(line.as_bytes())
    }

    pub fn broker_raw_bytes(&mut self, line: &[u8]) -> Value {
        self.broker.handle_line(line)
    }

    /// Sends a command that must succeed and returns the whole response.
    pub fn ok(&mut self, command: Value) -> Value {
        let response = self.send(command.clone());
        assert_eq!(response["ok"], json!(true), "{command} failed: {response}");
        response
    }

    /// Sends a command that must fail and returns its error code.
    pub fn err(&mut self, command: Value) -> String {
        let response = self.send(command.clone());
        assert_eq!(
            response["ok"],
            json!(false),
            "{command} succeeded: {response}"
        );
        response["error"]["code"].as_str().unwrap().to_owned()
    }

    pub fn tick(&mut self, seconds: u64) {
        self.ok(json!({"Command": "Tick", "Seconds": seconds}));
    }
}
