//! Shared driver for the integration tests: feeds command lines to a `Broker`.
#![allow(dead_code, reason = "each test binary uses a different subset")]

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
    pub fn send(&mut self, line: &str) -> Value {
        self.send_bytes(line.as_bytes())
    }

    pub fn send_bytes(&mut self, line: &[u8]) -> Value {
        let raw = self.broker.handle_line(line);
        assert!(!raw.contains('\n'), "response spans lines: {raw:?}");
        serde_json::from_str(&raw).unwrap()
    }

    /// Sends a command that must succeed and returns the response members
    /// without `ok`.
    pub fn ok(&mut self, line: &str) -> Value {
        let mut response = self.send(line);
        assert_eq!(response["ok"], true, "{line} -> {response}");
        response.as_object_mut().unwrap().remove("ok");
        response
    }

    /// Sends a command that must fail and returns the error code.
    pub fn err(&mut self, line: &str) -> String {
        let response = self.send(line);
        assert_eq!(response["ok"], false, "{line} -> {response}");
        assert!(response["error"]["message"].is_string(), "{response}");
        response["error"]["code"].as_str().unwrap().to_owned()
    }
}
