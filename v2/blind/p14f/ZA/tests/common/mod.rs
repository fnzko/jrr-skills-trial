//! Helpers shared by the integration tests.
#![allow(dead_code)]

use miniq::Broker;
use serde_json::Value;

/// A broker driven with JSON values instead of lines.
pub struct Harness {
    broker: Broker,
}

impl Harness {
    pub fn new() -> Self {
        Self {
            broker: Broker::new(),
        }
    }

    /// Sends one command and returns the parsed response.
    pub fn run(&mut self, command: Value) -> Value {
        let line = self.broker.handle_line(command.to_string().as_bytes());
        assert!(!line.contains('\n'), "response must be one line");
        serde_json::from_str(&line).expect("response is JSON")
    }

    /// Sends a command that must succeed and returns the response.
    pub fn ok(&mut self, command: Value) -> Value {
        let response = self.run(command.clone());
        assert_eq!(response["ok"], true, "{command} failed: {response}");
        response
    }

    /// Sends a command that must fail and returns the error code.
    pub fn err(&mut self, command: Value) -> String {
        let response = self.run(command.clone());
        assert_eq!(response["ok"], false, "{command} succeeded: {response}");
        assert!(response["error"]["message"].is_string());
        response["error"]["code"].as_str().unwrap().to_owned()
    }

    pub fn raw(&mut self, line: &[u8]) -> Value {
        serde_json::from_str(&self.broker.handle_line(line)).expect("response is JSON")
    }

    pub fn tick(&mut self, seconds: u64) {
        self.ok(serde_json::json!({"Command": "Tick", "Seconds": seconds}));
    }
}
