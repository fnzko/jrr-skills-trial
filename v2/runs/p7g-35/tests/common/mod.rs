//! Test helper: drives miniq through the same entry point as the binary.
#![allow(dead_code)]

use serde_json::{json, Value};

pub struct Session {
    service: miniq::Service,
}

impl Session {
    pub fn new() -> Self {
        Self {
            service: miniq::Service::new(),
        }
    }

    /// Sends one raw line and parses the single-line response.
    pub fn raw(&mut self, line: &str) -> Value {
        let out = miniq::handle_line(&mut self.service, line.as_bytes());
        assert!(!out.contains('\n'), "response must be one line: {out}");
        serde_json::from_str(&out).expect("response is JSON")
    }

    pub fn send(&mut self, request: Value) -> Value {
        self.raw(&request.to_string())
    }

    /// Sends a command and asserts success, returning the response.
    pub fn ok(&mut self, request: Value) -> Value {
        let r = self.send(request.clone());
        assert_eq!(r["ok"], true, "{request} failed: {r}");
        r
    }

    /// Sends a command and asserts it fails with `code`.
    pub fn err(&mut self, request: Value, code: &str) {
        let r = self.send(request.clone());
        assert_eq!(r["ok"], false, "{request} unexpectedly succeeded: {r}");
        assert_eq!(r["error"]["code"], code, "{request}: {r}");
        assert!(r["error"]["message"].is_string());
    }

    pub fn tick(&mut self, seconds: u64) {
        self.ok(json!({"Command": "Tick", "Seconds": seconds}));
    }
}
