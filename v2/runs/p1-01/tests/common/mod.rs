//! A session drives a fresh [`Broker`] through the same line protocol as the binary.

#![allow(
    dead_code,
    reason = "each test binary uses a different part of this module"
)]

use miniq::Broker;
use serde_json::{Value, json};

pub struct Session(Broker);

impl Session {
    pub fn new() -> Self {
        Self(Broker::new())
    }

    /// Sends one command and returns the whole parsed response.
    pub fn send(&mut self, command: Value) -> Value {
        let line = self.0.respond(command.to_string().as_bytes());
        serde_json::from_str(&line).expect("a response is one JSON value")
    }

    /// Sends a command that must succeed and returns its response.
    pub fn ok(&mut self, command: Value) -> Value {
        let response = self.send(command.clone());
        assert_eq!(
            response["ok"],
            json!(true),
            "command {command} failed: {response}"
        );
        response
    }

    /// Sends a command that must fail and returns the error code.
    pub fn err(&mut self, command: Value) -> String {
        let response = self.send(command.clone());
        assert_eq!(
            response["ok"],
            json!(false),
            "command {command} succeeded: {response}"
        );
        response["error"]["code"]
            .as_str()
            .unwrap_or_else(|| panic!("no error code in {response}"))
            .to_owned()
    }

    pub fn create(&mut self, name: &str) {
        self.ok(json!({"Command": "CreateQueue", "QueueName": name}));
    }

    pub fn create_with(&mut self, name: &str, attributes: Value) {
        self.ok(json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attributes}));
    }

    pub fn send_message(&mut self, queue: &str, body: &str) -> Value {
        self.ok(json!({"Command": "SendMessage", "QueueName": queue, "Body": body}))
    }

    pub fn send_grouped(&mut self, queue: &str, body: &str, group: &str, dedup: &str) -> Value {
        self.ok(json!({
            "Command": "SendMessage", "QueueName": queue, "Body": body,
            "GroupId": group, "DeduplicationId": dedup,
        }))
    }

    pub fn receive(&mut self, queue: &str) -> Value {
        self.receive_up_to(queue, 1)
    }

    pub fn receive_up_to(&mut self, queue: &str, max: u64) -> Value {
        self.ok(json!({"Command": "ReceiveMessage", "QueueName": queue, "MaxMessages": max}))
    }

    pub fn tick(&mut self, seconds: u64) {
        self.ok(json!({"Command": "Tick", "Seconds": seconds}));
    }

    pub fn counts(&mut self, queue: &str) -> Value {
        self.ok(json!({"Command": "GetQueueAttributes", "QueueName": queue}))["Counts"].clone()
    }

    pub fn delete_message(&mut self, queue: &str, handle: &str) -> Value {
        self.send(json!({"Command": "DeleteMessage", "QueueName": queue, "ReceiptHandle": handle}))
    }
}

pub fn counts(visible: u64, in_flight: u64, delayed: u64) -> Value {
    json!({"Visible": visible, "InFlight": in_flight, "Delayed": delayed})
}

/// The `Messages` of a receive response, reduced to `(MessageId, ReceiptHandle)` pairs.
pub fn delivered(response: &Value) -> Vec<(String, String)> {
    response["Messages"]
        .as_array()
        .expect("a receive response has Messages")
        .iter()
        .map(|m| {
            (
                m["MessageId"].as_str().unwrap().to_owned(),
                m["ReceiptHandle"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}
