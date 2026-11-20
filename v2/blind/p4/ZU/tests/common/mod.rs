//! A client for driving a `Service` with JSON values in tests.

// Each test binary compiles this module and uses only part of it.
#![allow(dead_code)]

use miniq::Service;
use serde_json::{Value, json};

#[derive(Default)]
pub struct Client {
    service: Service,
}

impl Client {
    /// Sends one request and returns the parsed response.
    pub fn call(&mut self, request: Value) -> Value {
        let line = self.service.handle_line(request.to_string().as_bytes());
        serde_json::from_str(&line).unwrap()
    }

    /// Sends one request that must fail, and returns its error code after
    /// checking that the response has the failure shape.
    #[track_caller]
    pub fn error(&mut self, request: Value) -> String {
        let response = self.call(request);
        let error = &response["error"];
        assert_eq!(
            response,
            json!({"ok": false, "error": {"code": error["code"], "message": error["message"]}}),
        );
        assert!(error["message"].is_string(), "message in {response}");
        error["code"].as_str().unwrap().to_owned()
    }

    #[track_caller]
    pub fn create(&mut self, name: &str, attributes: Value) {
        let response = self
            .call(json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attributes}));
        assert_eq!(response, json!({"ok": true}), "creating {name}");
    }

    #[track_caller]
    pub fn tick(&mut self, seconds: u32) {
        let response = self.call(json!({"Command": "Tick", "Seconds": seconds}));
        assert_eq!(response["ok"], json!(true));
    }

    /// Sends `body` to a standard queue and returns the `MessageId`.
    #[track_caller]
    pub fn send(&mut self, queue: &str, body: &str) -> String {
        let response =
            self.call(json!({"Command": "SendMessage", "QueueName": queue, "Body": body}));
        response["MessageId"].as_str().unwrap().to_owned()
    }

    /// Receives up to `max` messages and returns the `Messages` array.
    #[track_caller]
    pub fn receive(&mut self, queue: &str, max: u32) -> Vec<Value> {
        let response =
            self.call(json!({"Command": "ReceiveMessage", "QueueName": queue, "MaxMessages": max}));
        let Value::Array(messages) = &response["Messages"] else {
            panic!("no Messages in {response}");
        };
        messages.clone()
    }

    /// The `Counts` of a queue, as `[visible, in_flight, delayed]`.
    #[track_caller]
    pub fn counts(&mut self, queue: &str) -> [u64; 3] {
        let response = self.call(json!({"Command": "GetQueueAttributes", "QueueName": queue}));
        let counts = &response["Counts"];
        ["Visible", "InFlight", "Delayed"].map(|state| counts[state].as_u64().unwrap())
    }
}

/// The values of `member` in each message, as strings.
pub fn members(messages: &[Value], member: &str) -> Vec<String> {
    messages
        .iter()
        .map(|message| match &message[member] {
            Value::String(text) => text.clone(),
            other => other.to_string(),
        })
        .collect()
}
