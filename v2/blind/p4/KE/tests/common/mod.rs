#![allow(dead_code)] // Each test binary uses a different subset of these helpers.

use miniq::Service;
use serde_json::{Value, json};

/// A miniq service driven one JSON command at a time.
#[derive(Default)]
pub struct Session {
    service: Service,
}

impl Session {
    pub fn new() -> Self {
        Self::default()
    }

    /// Sends a raw line and returns the parsed response.
    pub fn raw(&mut self, line: &[u8]) -> Value {
        let response = self.service.handle_line(line);
        assert!(!response.contains('\n'), "response spans lines: {response}");
        serde_json::from_str(&response).expect("response is JSON")
    }

    /// Sends a command and returns the parsed response, success or not.
    pub fn call(&mut self, command: Value) -> Value {
        self.raw(command.to_string().as_bytes())
    }

    /// Sends a command that must succeed and returns its response without the `ok` member.
    pub fn ok(&mut self, command: Value) -> Value {
        let mut response = self.call(command.clone());
        assert_eq!(response["ok"], true, "{command} failed: {response}");
        response.as_object_mut().unwrap().remove("ok");
        response
    }

    /// Sends a command that must fail and returns its error code.
    pub fn err(&mut self, command: Value) -> String {
        let response = self.call(command.clone());
        assert_eq!(response["ok"], false, "{command} should fail: {response}");
        let error = &response["error"];
        assert!(
            error["message"].is_string(),
            "error without message: {response}"
        );
        error["code"]
            .as_str()
            .expect("string error code")
            .to_owned()
    }

    pub fn create(&mut self, name: &str, attributes: Value) {
        self.ok(json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attributes}));
    }

    pub fn tick(&mut self, seconds: u64) {
        self.ok(json!({"Command": "Tick", "Seconds": seconds}));
    }

    pub fn send(&mut self, queue: &str, body: &str) -> String {
        let response = self.ok(json!({"Command": "SendMessage", "QueueName": queue, "Body": body}));
        response["MessageId"].as_str().unwrap().to_owned()
    }

    pub fn send_fifo(&mut self, queue: &str, body: &str, group: &str) -> Value {
        self.ok(json!({
            "Command": "SendMessage", "QueueName": queue, "Body": body,
            "GroupId": group, "DeduplicationId": body,
        }))
    }

    pub fn receive(&mut self, queue: &str, max: u64) -> Vec<Value> {
        let response =
            self.ok(json!({"Command": "ReceiveMessage", "QueueName": queue, "MaxMessages": max}));
        response["Messages"].as_array().unwrap().clone()
    }

    pub fn receive_with_timeout(&mut self, queue: &str, max: u64, timeout: u64) -> Vec<Value> {
        let response = self.ok(json!({
            "Command": "ReceiveMessage", "QueueName": queue,
            "MaxMessages": max, "VisibilityTimeout": timeout,
        }));
        response["Messages"].as_array().unwrap().clone()
    }

    pub fn attributes(&mut self, queue: &str) -> Value {
        self.ok(json!({"Command": "GetQueueAttributes", "QueueName": queue}))
    }

    pub fn counts(&mut self, queue: &str) -> (u64, u64, u64) {
        let counts = &self.attributes(queue)["Counts"];
        let get = |name: &str| counts[name].as_u64().unwrap();
        (get("Visible"), get("InFlight"), get("Delayed"))
    }
}

pub fn bodies(messages: &[Value]) -> Vec<&str> {
    messages
        .iter()
        .map(|m| m["Body"].as_str().unwrap())
        .collect()
}

pub fn handle(message: &Value) -> &str {
    message["ReceiptHandle"].as_str().unwrap()
}
