//! A driver that runs commands against a fresh broker and parses the replies.

use miniq::broker::Broker;
use serde_json::{Value, json};

#[derive(Default)]
pub struct Session {
    broker: Broker,
}

impl Session {
    pub fn new() -> Self {
        Self::default()
    }

    /// Sends one command line and returns the parsed response.
    pub fn send(&mut self, line: &str) -> Value {
        self.send_bytes(line.as_bytes())
    }

    pub fn send_bytes(&mut self, line: &[u8]) -> Value {
        let response = self.broker.handle_line(line);
        serde_json::from_str(&response).expect("every response is one JSON value")
    }

    pub fn command(&mut self, command: Value) -> Value {
        self.send(&command.to_string())
    }

    /// Sends `command` and asserts that it succeeded with no result members.
    pub fn expect_ok(&mut self, command: Value) {
        assert_eq!(self.command(command), json!({"ok": true}));
    }

    pub fn create(&mut self, name: &str, attributes: Value) {
        self.expect_ok(
            json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attributes}),
        );
    }

    pub fn tick(&mut self, seconds: u64) {
        let response = self.command(json!({"Command": "Tick", "Seconds": seconds}));
        assert_eq!(response["ok"], json!(true));
    }
}

/// The `code` of a failure response, or a marker when it succeeded.
pub fn code(response: &Value) -> &str {
    if response["ok"] == json!(true) {
        return "<ok>";
    }
    response["error"]["code"]
        .as_str()
        .expect("a failure response carries an error code")
}
