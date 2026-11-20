#![allow(dead_code)] // Each test binary uses a different subset.

use miniq::{Broker, handle_line};
use serde_json::{Value, json};

/// One broker fed one line at a time, as the binary would be.
#[derive(Default)]
pub struct Session {
    broker: Broker,
}

impl Session {
    /// Runs `line` and returns its response. The free-text `error.message`
    /// is checked to be a string and then removed, so tests can compare the
    /// rest of the response whole.
    pub fn call(&mut self, line: &str) -> Value {
        let mut response: Value =
            serde_json::from_str(&handle_line(&mut self.broker, line.as_bytes())).unwrap();
        if let Some(error) = response.get_mut("error") {
            let message = error.as_object_mut().unwrap().remove("message").unwrap();
            assert!(message.is_string(), "message {message}");
        }
        response
    }

    /// Runs `line`, which must succeed without result members.
    #[track_caller]
    pub fn ok(&mut self, line: &str) {
        assert_eq!(self.call(line), ok(), "line {line}");
    }
}

pub fn ok() -> Value {
    json!({"ok": true})
}

pub fn err(code: &str) -> Value {
    json!({"ok": false, "error": {"code": code}})
}
