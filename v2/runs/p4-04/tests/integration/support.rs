//! Shared harness: drives a `Broker` one line at a time and checks replies.

use miniq::Broker;
use serde_json::{Value, json};

#[derive(Default)]
pub struct Session(Broker);

impl Session {
    /// Runs a raw line (without its newline) and returns the reply as JSON.
    pub fn line(&mut self, line: &[u8]) -> Value {
        serde_json::to_value(self.0.handle_line(line)).expect("a Response always serializes")
    }

    pub fn run(&mut self, command: Value) -> Value {
        self.line(command.to_string().as_bytes())
    }

    /// Runs `command` and asserts it fails with exactly `code`, a string
    /// message and no other top-level members.
    #[track_caller]
    pub fn fails(&mut self, command: Value, code: &str) {
        assert_error(&self.run(command), code);
    }
}

#[track_caller]
pub fn assert_error(reply: &Value, code: &str) {
    let message = &reply["error"]["message"];
    assert!(message.is_string(), "no string message in {reply}");
    assert_eq!(
        *reply,
        json!({"ok": false, "error": {"code": code, "message": message}})
    );
}

impl Session {
    #[track_caller]
    pub fn create(&mut self, name: &str, attributes: Value) {
        let command =
            json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attributes});
        assert_eq!(self.run(command), json!({"ok": true}));
    }

    #[track_caller]
    pub fn tick(&mut self, seconds: u32) {
        let reply = self.run(json!({"Command": "Tick", "Seconds": seconds}));
        assert_eq!(reply["ok"], json!(true), "{reply}");
    }

    /// Sends `body` to a standard queue and returns the reply.
    pub fn send(&mut self, queue: &str, body: &str) -> Value {
        self.run(json!({"Command": "SendMessage", "QueueName": queue, "Body": body}))
    }

    /// Receives up to ten messages with the queue's default timeout.
    pub fn receive(&mut self, queue: &str) -> Value {
        self.run(json!({"Command": "ReceiveMessage", "QueueName": queue, "MaxMessages": 10}))
    }

    /// The `MessageId`s returned by receiving up to ten messages.
    pub fn receive_ids(&mut self, queue: &str) -> Vec<String> {
        let reply = self.receive(queue);
        let messages = reply["Messages"]
            .as_array()
            .unwrap_or_else(|| panic!("{reply}"));
        messages
            .iter()
            .map(|message| {
                message["MessageId"]
                    .as_str()
                    .expect("MessageId is a string")
                    .to_owned()
            })
            .collect()
    }

    #[track_caller]
    pub fn delete(&mut self, queue: &str, handle: &str) {
        let command =
            json!({"Command": "DeleteMessage", "QueueName": queue, "ReceiptHandle": handle});
        assert_eq!(self.run(command), json!({"ok": true}));
    }

    pub fn counts(&mut self, queue: &str) -> Value {
        self.run(json!({"Command": "GetQueueAttributes", "QueueName": queue}))["Counts"].take()
    }
}

pub fn counts(visible: u64, in_flight: u64, delayed: u64) -> Value {
    json!({"Visible": visible, "InFlight": in_flight, "Delayed": delayed})
}
