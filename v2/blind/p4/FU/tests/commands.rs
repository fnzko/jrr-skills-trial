//! End-to-end behavior of each command, driven through the line protocol.

use miniq::{Service, handle_line};
use serde_json::{Value, json};

/// Feeds command lines to one service and checks the responses.
struct Session(Service);

impl Session {
    fn new() -> Self {
        Self(Service::new())
    }

    fn send(&mut self, command: Value) -> Value {
        self.raw(&command.to_string())
    }

    fn raw(&mut self, line: &str) -> Value {
        let response = handle_line(&mut self.0, line.as_bytes());
        assert!(!response.contains('\n'), "response must be one line");
        serde_json::from_str(&response).expect("response is JSON")
    }

    /// Runs a command that must succeed and returns its response.
    fn ok(&mut self, command: Value) -> Value {
        let response = self.send(command.clone());
        assert_eq!(response["ok"], true, "{command} failed: {response}");
        response
    }

    /// Runs a command that must fail and returns its error code.
    fn err(&mut self, command: Value) -> String {
        let response = self.send(command.clone());
        assert_eq!(response["ok"], false, "{command} succeeded: {response}");
        let error = &response["error"];
        assert!(error["message"].is_string());
        error["code"].as_str().expect("code is a string").to_owned()
    }

    fn create(&mut self, name: &str, attributes: Value) {
        self.ok(json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attributes}));
    }

    fn put(&mut self, queue: &str, body: &str) -> Value {
        self.ok(json!({"Command": "SendMessage", "QueueName": queue, "Body": body}))
    }

    fn put_fifo(&mut self, queue: &str, body: &str, group: &str, dedup: &str) -> Value {
        self.ok(json!({
            "Command": "SendMessage", "QueueName": queue, "Body": body,
            "GroupId": group, "DeduplicationId": dedup,
        }))
    }

    fn receive(&mut self, queue: &str, max: u64, timeout: Option<u64>) -> Vec<Value> {
        let mut command =
            json!({"Command": "ReceiveMessage", "QueueName": queue, "MaxMessages": max});
        if let Some(timeout) = timeout {
            command["VisibilityTimeout"] = timeout.into();
        }
        let response = self.ok(command);
        response["Messages"].as_array().expect("Messages").clone()
    }

    fn bodies(&mut self, queue: &str, max: u64, timeout: Option<u64>) -> Vec<String> {
        self.receive(queue, max, timeout)
            .iter()
            .map(|m| m["Body"].as_str().unwrap().to_owned())
            .collect()
    }

    fn tick(&mut self, seconds: u64) {
        self.ok(json!({"Command": "Tick", "Seconds": seconds}));
    }

    fn counts(&mut self, queue: &str) -> Value {
        self.ok(json!({"Command": "GetQueueAttributes", "QueueName": queue}))["Counts"].clone()
    }
}

#[test]
fn failure_responses_have_the_documented_shape() {
    let mut s = Session::new();
    let response = s.raw("");
    assert_eq!(response["ok"], false);
    assert_eq!(response["error"]["code"], "InvalidRequest");
    assert_eq!(response.as_object().unwrap().len(), 2);
    assert_eq!(s.raw("not json")["error"]["code"], "InvalidRequest");
    assert_eq!(
        s.raw(r#"{"Command":"Tick","Seconds":5}"#),
        json!({"ok": true, "Now": 5})
    );
}

#[test]
fn create_queue_defaults_and_idempotence() {
    let mut s = Session::new();
    assert_eq!(
        s.send(json!({"Command": "CreateQueue", "QueueName": "jobs"})),
        json!({"ok": true})
    );
    let response = s.ok(json!({"Command": "GetQueueAttributes", "QueueName": "jobs"}));
    assert_eq!(
        response,
        json!({
            "ok": true,
            "Attributes": {
                "VisibilityTimeout": 30, "DelaySeconds": 0, "RetentionSeconds": 345600,
                "Fifo": false, "ContentBasedDeduplication": false,
            },
            "Counts": {"Visible": 0, "InFlight": 0, "Delayed": 0},
        })
    );
    // Same attributes after defaults: succeeds and changes nothing.
    s.create("jobs", json!({"VisibilityTimeout": 30}));
    assert_eq!(
        s.err(json!({"Command": "CreateQueue", "QueueName": "jobs", "Attributes": {"DelaySeconds": 1}})),
        "QueueAlreadyExists"
    );
}

#[test]
fn create_queue_rules_in_order() {
    let mut s = Session::new();
    let create = |name: &str, attributes: Value| json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attributes});
    assert_eq!(s.err(create("a.fifo", json!({}))), "QueueNameInvalid");
    assert_eq!(
        s.err(create("a", json!({"Fifo": true}))),
        "QueueNameInvalid"
    );
    assert_eq!(
        s.err(create("a", json!({"ContentBasedDeduplication": true}))),
        "InvalidParameterValue"
    );
    assert_eq!(
        s.err(create("a", json!({"MaxReceiveCount": 3}))),
        "InvalidParameterValue"
    );
    assert_eq!(
        s.err(create(
            "a",
            json!({"DeadLetterQueue": "dlq", "MaxReceiveCount": 3})
        )),
        "QueueDoesNotExist"
    );
    s.create("dlq.fifo", json!({"Fifo": true}));
    assert_eq!(
        s.err(create(
            "a",
            json!({"DeadLetterQueue": "dlq.fifo", "MaxReceiveCount": 3})
        )),
        "InvalidParameterValue"
    );
    s.create("dlq", json!({}));
    s.create("a", json!({"DeadLetterQueue": "dlq", "MaxReceiveCount": 3}));
    s.create("a", json!({"DeadLetterQueue": "dlq", "MaxReceiveCount": 3}));
    assert_eq!(s.err(create("a", json!({}))), "QueueAlreadyExists");
}

#[test]
fn queue_limit() {
    let mut s = Session::new();
    for i in 0..1000 {
        s.create(&format!("q{i}"), json!({}));
    }
    assert_eq!(
        s.err(json!({"Command": "CreateQueue", "QueueName": "extra"})),
        "LimitExceeded"
    );
    // An identical existing queue is still accepted at the limit.
    s.create("q0", json!({}));
}

#[test]
fn list_queues_sorts_by_bytes_and_filters_by_prefix() {
    let mut s = Session::new();
    for name in ["b", "_x", "a-1", "A", "a.fifo", "0", "a"] {
        let fifo = name.ends_with(".fifo");
        s.create(name, json!({"Fifo": fifo}));
    }
    assert_eq!(
        s.ok(json!({"Command": "ListQueues"}))["QueueNames"],
        json!(["0", "A", "_x", "a", "a-1", "a.fifo", "b"])
    );
    assert_eq!(
        s.ok(json!({"Command": "ListQueues", "Prefix": "a"}))["QueueNames"],
        json!(["a", "a-1", "a.fifo"])
    );
}

#[test]
fn set_queue_attributes_rules() {
    let mut s = Session::new();
    s.create("q", json!({}));
    s.create("f.fifo", json!({"Fifo": true}));
    let set = |name: &str, attributes: Value| json!({"Command": "SetQueueAttributes", "QueueName": name, "Attributes": attributes});
    assert_eq!(
        s.err(json!({"Command": "SetQueueAttributes", "QueueName": "q"})),
        "MissingParameter"
    );
    assert_eq!(s.err(set("nope", json!({}))), "QueueDoesNotExist");
    assert_eq!(
        s.err(set("q", json!({"Fifo": false}))),
        "UnsupportedOperation"
    );
    assert_eq!(
        s.err(set("q", json!({"ContentBasedDeduplication": true}))),
        "InvalidParameterValue"
    );
    assert_eq!(
        s.err(set("q", json!({"DeadLetterQueue": "q"}))),
        "InvalidParameterValue"
    );
    assert_eq!(
        s.err(set(
            "q",
            json!({"DeadLetterQueue": "x", "MaxReceiveCount": 1})
        )),
        "QueueDoesNotExist"
    );
    assert_eq!(
        s.err(set(
            "q",
            json!({"DeadLetterQueue": "q", "MaxReceiveCount": 1})
        )),
        "InvalidParameterValue"
    );
    assert_eq!(
        s.err(set(
            "q",
            json!({"DeadLetterQueue": "f.fifo", "MaxReceiveCount": 1})
        )),
        "InvalidParameterValue"
    );

    s.create("dlq", json!({}));
    s.ok(set(
        "q",
        json!({"VisibilityTimeout": 5, "DeadLetterQueue": "dlq", "MaxReceiveCount": 2}),
    ));
    s.ok(set("q", json!({})));
    let attributes =
        s.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}))["Attributes"].clone();
    assert_eq!(attributes["VisibilityTimeout"], 5);
    assert_eq!(attributes["DeadLetterQueue"], "dlq");
    assert_eq!(attributes["MaxReceiveCount"], 2);
}

#[test]
fn delete_queue_clears_dead_letter_configurations() {
    let mut s = Session::new();
    s.create("dlq", json!({}));
    s.create("q", json!({"DeadLetterQueue": "dlq", "MaxReceiveCount": 1}));
    s.put("q", "x");
    s.ok(json!({"Command": "DeleteQueue", "QueueName": "dlq"}));
    let attributes =
        s.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}))["Attributes"].clone();
    assert!(attributes.get("DeadLetterQueue").is_none());
    assert!(attributes.get("MaxReceiveCount").is_none());
    assert_eq!(
        s.err(json!({"Command": "DeleteQueue", "QueueName": "dlq"})),
        "QueueDoesNotExist"
    );
    // Receiving past the old limit no longer moves anything.
    s.receive("q", 1, Some(0));
    assert_eq!(s.receive("q", 1, Some(0))[0]["ReceiveCount"], 2);
}

#[test]
fn recreated_queue_shares_nothing() {
    let mut s = Session::new();
    s.create("q", json!({}));
    s.put("q", "x");
    let handle = s.receive("q", 1, None)[0]["ReceiptHandle"].clone();
    s.ok(json!({"Command": "DeleteQueue", "QueueName": "q"}));
    s.create("q", json!({}));
    assert_eq!(
        s.counts("q"),
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(
        s.err(json!({"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": handle})),
        "ReceiptHandleExpired"
    );
}

#[test]
fn send_and_receive_standard_messages() {
    let mut s = Session::new();
    s.create("q", json!({}));
    assert_eq!(s.put("q", "hello"), json!({"ok": true, "MessageId": "m-1"}));
    s.tick(7);
    let messages = s.receive("q", 10, None);
    assert_eq!(
        messages,
        [json!({
            "MessageId": "m-1", "ReceiptHandle": "r-1", "Body": "hello",
            "ReceiveCount": 1, "SentAt": 0, "FirstReceivedAt": 7,
        })]
    );
    assert_eq!(
        s.counts("q"),
        json!({"Visible": 0, "InFlight": 1, "Delayed": 0})
    );
    s.tick(30);
    let again = s.receive("q", 10, None);
    assert_eq!(again[0]["ReceiptHandle"], "r-2");
    assert_eq!(again[0]["ReceiveCount"], 2);
    assert_eq!(again[0]["FirstReceivedAt"], 7);
}

#[test]
fn message_ids_skip_failed_and_duplicate_sends() {
    let mut s = Session::new();
    s.create("q", json!({}));
    s.create("f.fifo", json!({"Fifo": true}));
    s.put("q", "a");
    s.err(json!({"Command": "SendMessage", "QueueName": "q", "Body": "b", "GroupId": "g"}));
    s.put_fifo("f.fifo", "x", "g", "d");
    assert_eq!(
        s.put_fifo("f.fifo", "y", "h", "d"),
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 1})
    );
    assert_eq!(s.put("q", "c")["MessageId"], "m-3");
}

#[test]
fn send_message_rules() {
    let mut s = Session::new();
    s.create("q", json!({}));
    s.create("f.fifo", json!({"Fifo": true}));
    s.create(
        "c.fifo",
        json!({"Fifo": true, "ContentBasedDeduplication": true}),
    );
    let send = |queue: &str, extra: Value| {
        let mut command = json!({"Command": "SendMessage", "QueueName": queue, "Body": "b"});
        for (k, v) in extra.as_object().unwrap() {
            command[k] = v.clone();
        }
        command
    };
    assert_eq!(
        s.err(send("q", json!({"DeduplicationId": "d"}))),
        "UnsupportedOperation"
    );
    assert_eq!(
        s.err(send("f.fifo", json!({"DelaySeconds": 0, "GroupId": "g"}))),
        "UnsupportedOperation"
    );
    assert_eq!(
        s.err(send("f.fifo", json!({"DeduplicationId": "d"}))),
        "MissingParameter"
    );
    assert_eq!(
        s.err(send("f.fifo", json!({"GroupId": "g"}))),
        "MissingParameter"
    );
    assert_eq!(
        s.ok(send("c.fifo", json!({"GroupId": "g"})))["SequenceNumber"],
        1
    );
    // Content-based deduplication uses the body, whatever the group.
    assert_eq!(
        s.ok(send("c.fifo", json!({"GroupId": "other"}))),
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
    assert_eq!(
        s.err(send("nope", json!({"GroupId": "g"}))),
        "QueueDoesNotExist"
    );
}

#[test]
fn deduplication_window_outlives_the_message() {
    let mut s = Session::new();
    s.create("f.fifo", json!({"Fifo": true}));
    s.put_fifo("f.fifo", "x", "g", "d");
    s.ok(json!({"Command": "PurgeQueue", "QueueName": "f.fifo"}));
    s.tick(299);
    assert_eq!(s.put_fifo("f.fifo", "y", "g", "d")["MessageId"], "m-1");
    assert_eq!(s.counts("f.fifo")["Visible"], 0);
    s.tick(1);
    assert_eq!(
        s.put_fifo("f.fifo", "y", "g", "d"),
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2})
    );
}

#[test]
fn delays_from_queue_and_message() {
    let mut s = Session::new();
    s.create("q", json!({"DelaySeconds": 5}));
    s.put("q", "queue-delay");
    s.ok(json!({"Command": "SendMessage", "QueueName": "q", "Body": "none", "DelaySeconds": 0}));
    s.ok(json!({"Command": "SendMessage", "QueueName": "q", "Body": "long", "DelaySeconds": 10}));
    assert_eq!(
        s.counts("q"),
        json!({"Visible": 1, "InFlight": 0, "Delayed": 2})
    );
    s.tick(5);
    assert_eq!(s.bodies("q", 10, None), ["queue-delay", "none"]);
    s.tick(5);
    assert_eq!(s.bodies("q", 10, None), ["long"]);
}

#[test]
fn fifo_groups_and_sequence_numbers() {
    let mut s = Session::new();
    s.create("f.fifo", json!({"Fifo": true}));
    s.put_fifo("f.fifo", "a1", "a", "1");
    s.put_fifo("f.fifo", "a2", "a", "2");
    s.put_fifo("f.fifo", "b1", "b", "3");
    let messages = s.receive("f.fifo", 10, None);
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0]["GroupId"], "a");
    assert_eq!(messages[0]["SequenceNumber"], 1);
    assert_eq!(messages[1]["Body"], "b1");
    assert_eq!(messages[1]["SequenceNumber"], 3);
    let handle = messages[0]["ReceiptHandle"].clone();
    s.ok(json!({"Command": "DeleteMessage", "QueueName": "f.fifo", "ReceiptHandle": handle}));
    assert_eq!(s.bodies("f.fifo", 10, None), ["a2"]);
}

#[test]
fn receipt_handle_checks() {
    let mut s = Session::new();
    s.create("q", json!({}));
    s.create("other", json!({}));
    s.put("q", "x");
    let delete = |queue: &str, handle: &str| json!({"Command": "DeleteMessage", "QueueName": queue, "ReceiptHandle": handle});
    assert_eq!(s.err(delete("q", "r-1")), "ReceiptHandleInvalid");
    s.receive("q", 1, None);
    for never in ["r-01", "R-1", "r-1 ", "r-2"] {
        assert_eq!(s.err(delete("q", never)), "ReceiptHandleInvalid", "{never}");
    }
    assert_eq!(s.err(delete("nope", "r-1")), "QueueDoesNotExist");
    assert_eq!(s.err(delete("other", "r-1")), "ReceiptHandleExpired");
    s.ok(delete("q", "r-1"));
    assert_eq!(s.err(delete("q", "r-1")), "ReceiptHandleExpired");
}

#[test]
fn change_message_visibility() {
    let mut s = Session::new();
    s.create("q", json!({}));
    s.put("q", "x");
    s.receive("q", 1, None);
    let change = |timeout: u64| {
        json!({"Command": "ChangeMessageVisibility", "QueueName": "q",
               "ReceiptHandle": "r-1", "VisibilityTimeout": timeout})
    };
    assert_eq!(
        s.err(
            json!({"Command": "ChangeMessageVisibility", "QueueName": "q", "ReceiptHandle": "r-1"})
        ),
        "MissingParameter"
    );
    s.tick(20);
    s.ok(change(100));
    s.tick(99);
    assert!(s.receive("q", 1, None).is_empty());
    s.ok(change(0));
    assert_eq!(s.err(change(10)), "ReceiptHandleExpired");
    assert_eq!(s.receive("q", 1, None)[0]["ReceiveCount"], 2);
}

#[test]
fn dead_letter_moves() {
    let mut s = Session::new();
    s.create("dlq", json!({"RetentionSeconds": 100}));
    s.create("q", json!({"DeadLetterQueue": "dlq", "MaxReceiveCount": 1}));
    s.put("q", "old");
    s.tick(50);
    s.put("q", "new");
    s.receive("q", 10, Some(0));
    s.tick(60);
    // Both reached MaxReceiveCount: they move instead of being delivered, and "old" (sent at 0)
    // is already past the dead-letter queue's retention, so it is removed on arrival.
    assert!(s.receive("q", 10, None).is_empty());
    assert_eq!(
        s.counts("q"),
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
    let moved = s.receive("dlq", 10, None);
    assert_eq!(moved.len(), 1);
    assert_eq!(moved[0]["Body"], "new");
    assert_eq!(moved[0]["MessageId"], "m-2");
    assert_eq!(moved[0]["SentAt"], 50);
    assert_eq!(moved[0]["ReceiveCount"], 1);
    assert_eq!(moved[0]["FirstReceivedAt"], 110);
}

#[test]
fn fifo_dead_letter_takes_new_sequence_number() {
    let mut s = Session::new();
    s.create("dlq.fifo", json!({"Fifo": true}));
    s.create(
        "f.fifo",
        json!({"Fifo": true, "DeadLetterQueue": "dlq.fifo", "MaxReceiveCount": 1}),
    );
    s.put_fifo("dlq.fifo", "native", "z", "n");
    s.put_fifo("f.fifo", "a1", "a", "1");
    s.put_fifo("f.fifo", "a2", "a", "2");
    s.receive("f.fifo", 1, Some(0));
    // a1 moves; a2 becomes its group's head and is delivered by the same call.
    let delivered = s.receive("f.fifo", 1, None);
    assert_eq!(delivered[0]["Body"], "a2");
    let moved = s.receive("dlq.fifo", 10, None);
    assert_eq!(moved[1]["Body"], "a1");
    assert_eq!(moved[1]["GroupId"], "a");
    assert_eq!(moved[1]["SequenceNumber"], 2);
}

#[test]
fn retention_and_purge() {
    let mut s = Session::new();
    s.create("q", json!({"RetentionSeconds": 60}));
    s.put("q", "a");
    s.tick(30);
    s.put("q", "b");
    s.tick(30);
    assert_eq!(s.counts("q")["Visible"], 1);
    s.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q",
                "Attributes": {"RetentionSeconds": 60}}));
    s.tick(29);
    s.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q",
                "Attributes": {"RetentionSeconds": 1000}}));
    s.put("q", "c");
    s.receive("q", 1, None);
    s.ok(json!({"Command": "PurgeQueue", "QueueName": "q"}));
    assert_eq!(
        s.counts("q"),
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn message_limit_per_queue() {
    let mut s = Session::new();
    s.create("q", json!({}));
    for _ in 0..100_000 {
        s.put("q", "x");
    }
    assert_eq!(
        s.err(json!({"Command": "SendMessage", "QueueName": "q", "Body": "x"})),
        "LimitExceeded"
    );
    assert_eq!(s.counts("q")["Visible"], 100_000);
    s.receive("q", 10, None);
    assert_eq!(s.counts("q")["InFlight"], 10);
}
