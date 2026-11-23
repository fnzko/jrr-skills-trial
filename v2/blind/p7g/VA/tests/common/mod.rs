#![allow(dead_code)]

use miniq::broker::Broker;
use serde_json::{Value, json};

/// Drives a broker in process, one command at a time.
pub struct Session {
    broker: Broker,
}

impl Session {
    pub fn new() -> Session {
        Session {
            broker: Broker::new(),
        }
    }

    pub fn raw(&mut self, line: &[u8]) -> Value {
        let text = self.broker.handle_line(line);
        serde_json::from_str(&text).expect("the broker writes one JSON value per line")
    }

    pub fn call(&mut self, command: Value) -> Value {
        self.raw(command.to_string().as_bytes())
    }

    /// Runs a command that must succeed and returns the response without `ok`.
    pub fn ok(&mut self, command: Value) -> Value {
        let mut response = self.call(command.clone());
        assert_eq!(response["ok"], json!(true), "{command} failed: {response}");
        response.as_object_mut().unwrap().remove("ok");
        response
    }

    /// Runs a command that must fail and returns the error code.
    pub fn err(&mut self, command: Value) -> String {
        let response = self.call(command.clone());
        assert_eq!(
            response["ok"],
            json!(false),
            "{command} succeeded: {response}"
        );
        response["error"]["code"].as_str().unwrap().to_owned()
    }

    pub fn create(&mut self, name: &str, attributes: Value) {
        self.ok(json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attributes}));
    }

    pub fn send(&mut self, queue: &str, body: &str) -> Value {
        self.ok(json!({"Command": "SendMessage", "QueueName": queue, "Body": body}))
    }

    pub fn receive(&mut self, queue: &str) -> Vec<Value> {
        self.receive_up_to(queue, 1)
    }

    pub fn receive_up_to(&mut self, queue: &str, max: u32) -> Vec<Value> {
        let response =
            self.ok(json!({"Command": "ReceiveMessage", "QueueName": queue, "MaxMessages": max}));
        response["Messages"].as_array().unwrap().clone()
    }

    pub fn tick(&mut self, seconds: u32) {
        self.ok(json!({"Command": "Tick", "Seconds": seconds}));
    }

    pub fn counts(&mut self, queue: &str) -> Value {
        self.ok(json!({"Command": "GetQueueAttributes", "QueueName": queue}))["Counts"].clone()
    }
}

/// The `Body` of each message, in order.
pub fn bodies(messages: &[Value]) -> Vec<&str> {
    messages
        .iter()
        .map(|m| m["Body"].as_str().unwrap())
        .collect()
}
