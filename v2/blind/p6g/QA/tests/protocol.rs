//! End-to-end scenarios through `Broker::handle_line`.

use miniq::Broker;
use serde_json::{Value, json};

struct Session(Broker);

impl Session {
    fn new() -> Self {
        Self(Broker::new())
    }

    fn run(&mut self, command: Value) -> Value {
        self.raw(command.to_string().as_bytes())
    }

    fn raw(&mut self, line: &[u8]) -> Value {
        let response = self.0.handle_line(line);
        serde_json::from_str(&response).expect("response is JSON")
    }

    fn code(&mut self, command: Value) -> String {
        let response = self.run(command);
        assert_eq!(response["ok"], json!(false), "{response}");
        response["error"]["code"].as_str().expect("code").to_owned()
    }

    fn create(&mut self, name: &str, attributes: Value) {
        let response = self
            .run(json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attributes}));
        assert_eq!(response, json!({"ok": true}));
    }

    fn send(&mut self, queue: &str, body: &str) -> Value {
        self.run(json!({"Command": "SendMessage", "QueueName": queue, "Body": body}))
    }

    fn receive(&mut self, queue: &str, max: u32) -> Vec<Value> {
        let response =
            self.run(json!({"Command": "ReceiveMessage", "QueueName": queue, "MaxMessages": max}));
        response["Messages"].as_array().expect("Messages").clone()
    }

    fn tick(&mut self, seconds: u32) {
        let response = self.run(json!({"Command": "Tick", "Seconds": seconds}));
        assert_eq!(response["ok"], json!(true));
    }

    fn counts(&mut self, queue: &str) -> Value {
        self.run(json!({"Command": "GetQueueAttributes", "QueueName": queue}))["Counts"].clone()
    }
}

#[test]
fn request_shape_errors_are_invalid_request() {
    let mut s = Session::new();
    let lines: [&[u8]; 8] = [
        b"",
        b"not json",
        b"[1]",
        b"{\"Command\": 1}",
        b"{}",
        b"{\"Command\": \"Nope\"}",
        b"{\"Command\": \"Tick\", \"Seconds\": 1} trailing",
        b"{\"Command\": \"Tick\", \"Seconds\": 1}\xff",
    ];
    for line in lines {
        let response = s.raw(line);
        assert_eq!(
            response["error"]["code"],
            json!("InvalidRequest"),
            "{line:?}"
        );
    }
}

#[test]
fn tick_advances_the_clock_and_null_counts_as_absent() {
    let mut s = Session::new();
    assert_eq!(
        s.run(json!({"Command": "Tick", "Seconds": 5})),
        json!({"ok": true, "Now": 5})
    );
    assert_eq!(
        s.run(json!({"Command": "Tick", "Seconds": 0, "Extra": 1})),
        json!({"ok": true, "Now": 5})
    );
    assert_eq!(
        s.code(json!({"Command": "Tick", "Seconds": null})),
        "MissingParameter"
    );
    for bad in [
        json!(30.0),
        json!("1"),
        json!(-1),
        json!(1209601),
        json!(true),
    ] {
        assert_eq!(
            s.code(json!({"Command": "Tick", "Seconds": bad})),
            "InvalidParameterValue"
        );
    }
    assert_eq!(
        s.code(serde_json::from_str(r#"{"Command":"Tick","Seconds":3e1}"#).unwrap()),
        "InvalidParameterValue"
    );
}

#[test]
fn error_precedence_follows_the_spec() {
    let mut s = Session::new();
    // Missing beats a bad value elsewhere.
    assert_eq!(
        s.code(json!({"Command": "SendMessage", "Body": 5})),
        "MissingParameter"
    );
    // A bad value beats a missing queue.
    assert_eq!(
        s.code(json!({"Command": "SendMessage", "QueueName": "nope", "Body": ""})),
        "InvalidParameterValue"
    );
    // Queue lookup beats the command's own rules, whatever the name looks like.
    assert_eq!(
        s.code(json!({"Command": "SendMessage", "QueueName": "!?", "Body": "x"})),
        "QueueDoesNotExist"
    );
    // CreateQueue checks the name before the other values.
    let response =
        s.run(json!({"Command": "CreateQueue", "QueueName": "a b", "Attributes": {"Fifo": "x"}}));
    assert_eq!(response["error"]["code"], json!("QueueNameInvalid"));
    assert_eq!(
        s.code(json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": {"Bogus": 1}})),
        "InvalidParameterValue"
    );
    assert_eq!(
        s.code(json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": 3})),
        "InvalidParameterValue"
    );
    assert_eq!(
        s.code(json!({"Command": "SetQueueAttributes", "QueueName": "q"})),
        "MissingParameter"
    );
}

#[test]
fn create_queue_rules_in_order() {
    let mut s = Session::new();
    assert_eq!(
        s.code(json!({"Command": "CreateQueue", "QueueName": "q.fifo"})),
        "QueueNameInvalid"
    );
    assert_eq!(
        s.code(json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": {"Fifo": true}})),
        "QueueNameInvalid"
    );
    assert_eq!(s.code(json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": {"ContentBasedDeduplication": true}})), "InvalidParameterValue");
    assert_eq!(s.code(json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": {"MaxReceiveCount": 2}})), "InvalidParameterValue");
    assert_eq!(s.code(json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": {"MaxReceiveCount": 2, "DeadLetterQueue": "dlq"}})), "QueueDoesNotExist");
    s.create("dlq.fifo", json!({"Fifo": true}));
    assert_eq!(s.code(json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": {"MaxReceiveCount": 2, "DeadLetterQueue": "dlq.fifo"}})), "InvalidParameterValue");
}

#[test]
fn create_queue_is_idempotent_only_for_equal_attributes() {
    let mut s = Session::new();
    s.create("q", json!({"VisibilityTimeout": 30}));
    s.create("q", json!(null));
    s.create("q", json!({}));
    assert_eq!(
        s.code(
            json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": {"DelaySeconds": 1}})
        ),
        "QueueAlreadyExists"
    );
}

#[test]
fn queue_attributes_defaults_and_updates() {
    let mut s = Session::new();
    s.create("q", json!({}));
    let expected = json!({"ok": true,
        "Attributes": {"VisibilityTimeout": 30, "DelaySeconds": 0, "RetentionSeconds": 345600, "Fifo": false, "ContentBasedDeduplication": false},
        "Counts": {"Visible": 0, "InFlight": 0, "Delayed": 0}});
    assert_eq!(
        s.run(json!({"Command": "GetQueueAttributes", "QueueName": "q"})),
        expected
    );
    s.create("dlq", json!({}));
    let set = s.run(json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"DelaySeconds": 7, "DeadLetterQueue": "dlq", "MaxReceiveCount": 3}}));
    assert_eq!(set, json!({"ok": true}));
    let got = s.run(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(got["Attributes"]["DelaySeconds"], json!(7));
    assert_eq!(got["Attributes"]["DeadLetterQueue"], json!("dlq"));
    assert_eq!(got["Attributes"]["MaxReceiveCount"], json!(3));
}

#[test]
fn set_queue_attributes_rules() {
    let mut s = Session::new();
    s.create("q", json!({}));
    s.create("f.fifo", json!({"Fifo": true}));
    let set = |attrs: Value| json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": attrs});
    assert_eq!(s.code(set(json!({"Fifo": false}))), "UnsupportedOperation");
    assert_eq!(
        s.code(set(json!({"ContentBasedDeduplication": true}))),
        "InvalidParameterValue"
    );
    assert_eq!(
        s.code(set(json!({"MaxReceiveCount": 1}))),
        "InvalidParameterValue"
    );
    assert_eq!(
        s.code(set(json!({"MaxReceiveCount": 1, "DeadLetterQueue": "zz"}))),
        "QueueDoesNotExist"
    );
    assert_eq!(
        s.code(set(json!({"MaxReceiveCount": 1, "DeadLetterQueue": "q"}))),
        "InvalidParameterValue"
    );
    assert_eq!(
        s.code(set(
            json!({"MaxReceiveCount": 1, "DeadLetterQueue": "f.fifo"})
        )),
        "InvalidParameterValue"
    );
    assert_eq!(s.run(set(json!({}))), json!({"ok": true}));
}

#[test]
fn list_queues_sorts_bytewise_and_filters_by_prefix() {
    let mut s = Session::new();
    for name in ["b", "a.fifo", "_x", "B", "a-1", "a0", "a"] {
        let fifo = name.ends_with(".fifo");
        s.create(name, json!({"Fifo": fifo}));
    }
    let all = s.run(json!({"Command": "ListQueues"}));
    assert_eq!(
        all["QueueNames"],
        json!(["B", "_x", "a", "a-1", "a.fifo", "a0", "b"])
    );
    let a = s.run(json!({"Command": "ListQueues", "Prefix": "a"}));
    assert_eq!(a["QueueNames"], json!(["a", "a-1", "a.fifo", "a0"]));
}

#[test]
fn delete_queue_clears_dead_letter_references_and_forgets_messages() {
    let mut s = Session::new();
    s.create("dlq", json!({}));
    s.create("q", json!({"DeadLetterQueue": "dlq", "MaxReceiveCount": 2}));
    s.send("dlq", "old");
    assert_eq!(
        s.run(json!({"Command": "DeleteQueue", "QueueName": "dlq"})),
        json!({"ok": true})
    );
    let got = s.run(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(got["Attributes"].get("DeadLetterQueue"), None);
    assert_eq!(got["Attributes"].get("MaxReceiveCount"), None);
    s.create("dlq", json!({}));
    assert_eq!(
        s.counts("dlq"),
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(
        s.code(json!({"Command": "DeleteQueue", "QueueName": "gone"})),
        "QueueDoesNotExist"
    );
}

#[test]
fn send_receive_delete_cycle_with_visibility_and_delay() {
    let mut s = Session::new();
    s.create("q", json!({"VisibilityTimeout": 10}));
    assert_eq!(s.send("q", "one"), json!({"ok": true, "MessageId": "m-1"}));
    let delayed = s
        .run(json!({"Command": "SendMessage", "QueueName": "q", "Body": "two", "DelaySeconds": 5}));
    assert_eq!(delayed["MessageId"], json!("m-2"));
    assert_eq!(
        s.counts("q"),
        json!({"Visible": 1, "InFlight": 0, "Delayed": 1})
    );

    let first = s.receive("q", 10);
    assert_eq!(
        first,
        vec![
            json!({"MessageId": "m-1", "ReceiptHandle": "r-1", "Body": "one", "ReceiveCount": 1, "SentAt": 0, "FirstReceivedAt": 0})
        ]
    );
    assert!(s.receive("q", 10).is_empty());
    s.tick(5);
    let second = s.receive("q", 10);
    assert_eq!(second[0]["MessageId"], json!("m-2"));
    assert_eq!(second[0]["ReceiptHandle"], json!("r-2"));
    s.tick(5);
    // m-1's timeout ended at 10: redelivered with a new handle, old one expired.
    let again = s.receive("q", 10);
    assert_eq!(again[0]["ReceiveCount"], json!(2));
    assert_eq!(again[0]["FirstReceivedAt"], json!(0));
    assert_eq!(again[0]["ReceiptHandle"], json!("r-3"));
    let delete =
        |h: &str| json!({"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": h});
    assert_eq!(s.code(delete("r-1")), "ReceiptHandleExpired");
    assert_eq!(s.run(delete("r-3")), json!({"ok": true}));
    assert_eq!(s.code(delete("r-3")), "ReceiptHandleExpired");
    for never in ["r-9", "r-01", "R-1", "r-1 ", "x"] {
        assert_eq!(s.code(delete(never)), "ReceiptHandleInvalid", "{never}");
    }
}

#[test]
fn change_message_visibility_and_zero_timeout() {
    let mut s = Session::new();
    s.create("q", json!({}));
    s.send("q", "a");
    s.receive("q", 1);
    let change = |t: u32| json!({"Command": "ChangeMessageVisibility", "QueueName": "q", "ReceiptHandle": "r-1", "VisibilityTimeout": t});
    assert_eq!(s.run(change(100)), json!({"ok": true}));
    s.tick(99);
    assert_eq!(s.counts("q")["InFlight"], json!(1));
    s.tick(1);
    assert_eq!(s.counts("q")["Visible"], json!(1));
    assert_eq!(s.code(change(0)), "ReceiptHandleExpired");

    let r = s.run(json!({"Command": "ReceiveMessage", "QueueName": "q", "VisibilityTimeout": 0}));
    assert_eq!(r["Messages"][0]["ReceiptHandle"], json!("r-2"));
    assert_eq!(s.counts("q")["Visible"], json!(1));
    assert_eq!(
        s.code(json!({"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": "r-2"})),
        "ReceiptHandleExpired"
    );
}

#[test]
fn handle_from_another_queue_is_expired() {
    let mut s = Session::new();
    s.create("a", json!({}));
    s.create("b", json!({}));
    s.send("a", "x");
    s.receive("a", 1);
    let code =
        s.code(json!({"Command": "DeleteMessage", "QueueName": "b", "ReceiptHandle": "r-1"}));
    assert_eq!(code, "ReceiptHandleExpired");
}

#[test]
fn retention_removes_messages_and_lowering_it_acts_at_once() {
    let mut s = Session::new();
    s.create("q", json!({"RetentionSeconds": 100}));
    s.send("q", "a");
    s.tick(99);
    assert_eq!(s.counts("q")["Visible"], json!(1));
    s.tick(1);
    assert_eq!(s.counts("q")["Visible"], json!(0));

    s.create("r", json!({"RetentionSeconds": 1000}));
    s.send("r", "old");
    s.tick(60);
    s.send("r", "new");
    let set = |t: u32| json!({"Command": "SetQueueAttributes", "QueueName": "r", "Attributes": {"RetentionSeconds": t}});
    s.run(set(60));
    s.run(set(1000));
    // "old" is gone for good although retention was raised again.
    let left = s.receive("r", 10);
    assert_eq!(left.len(), 1);
    assert_eq!(left[0]["Body"], json!("new"));
}

#[test]
fn purge_keeps_sequence_numbers_and_deduplication() {
    let mut s = Session::new();
    s.create("q.fifo", json!({"Fifo": true}));
    let send = |id: &str| json!({"Command": "SendMessage", "QueueName": "q.fifo", "Body": "b", "GroupId": "g", "DeduplicationId": id});
    assert_eq!(
        s.run(send("d1")),
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
    s.run(json!({"Command": "PurgeQueue", "QueueName": "q.fifo"}));
    assert_eq!(
        s.run(send("d1")),
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
    assert_eq!(
        s.run(send("d2")),
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2})
    );
}

#[test]
fn send_message_rules_for_standard_and_fifo_queues() {
    let mut s = Session::new();
    s.create("std", json!({}));
    s.create("f.fifo", json!({"Fifo": true}));
    let send = |q: &str, extra: Value| {
        let mut m = json!({"Command": "SendMessage", "QueueName": q, "Body": "b"});
        m.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        m
    };
    assert_eq!(
        s.code(send("std", json!({"GroupId": "g"}))),
        "UnsupportedOperation"
    );
    assert_eq!(
        s.code(send("std", json!({"DeduplicationId": "g"}))),
        "UnsupportedOperation"
    );
    assert_eq!(
        s.code(send("f.fifo", json!({"DelaySeconds": 0, "GroupId": "g"}))),
        "UnsupportedOperation"
    );
    assert_eq!(s.code(send("f.fifo", json!({}))), "MissingParameter");
    assert_eq!(
        s.code(send("f.fifo", json!({"GroupId": "g"}))),
        "MissingParameter"
    );
    assert_eq!(
        s.code(send(
            "f.fifo",
            json!({"GroupId": "bad id", "DeduplicationId": "d"})
        )),
        "InvalidParameterValue"
    );
    assert_eq!(
        s.code(send("std", json!({"DelaySeconds": 901}))),
        "InvalidParameterValue"
    );
    let long_body = "x".repeat(1025);
    assert_eq!(
        s.code(json!({"Command": "SendMessage", "QueueName": "std", "Body": long_body})),
        "InvalidParameterValue"
    );
    let max_body = "é".repeat(512);
    assert_eq!(s.send("std", &max_body)["ok"], json!(true));
    // Failed sends use no message id.
    assert_eq!(s.send("std", "next")["MessageId"], json!("m-2"));
}

#[test]
fn deduplication_window_is_300_seconds_and_not_extended() {
    let mut s = Session::new();
    s.create(
        "q.fifo",
        json!({"Fifo": true, "ContentBasedDeduplication": true}),
    );
    let send = |body: &str| json!({"Command": "SendMessage", "QueueName": "q.fifo", "Body": body, "GroupId": "g"});
    assert_eq!(s.run(send("same"))["MessageId"], json!("m-1"));
    s.tick(299);
    assert_eq!(
        s.run(send("same")),
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
    s.tick(1);
    assert_eq!(
        s.run(send("same")),
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2})
    );
    // Ids are per queue and independent of the explicit id.
    s.create("p.fifo", json!({"Fifo": true}));
    let other = json!({"Command": "SendMessage", "QueueName": "p.fifo", "Body": "same", "GroupId": "g", "DeduplicationId": "same"});
    assert_eq!(s.run(other)["MessageId"], json!("m-3"));
}

#[test]
fn fifo_delivers_one_message_per_group_in_order() {
    let mut s = Session::new();
    s.create("q.fifo", json!({"Fifo": true, "VisibilityTimeout": 10}));
    let send = |s: &mut Session, g: &str, d: &str| {
        s.run(json!({"Command": "SendMessage", "QueueName": "q.fifo", "Body": d, "GroupId": g, "DeduplicationId": d}))
    };
    for (g, d) in [("a", "a1"), ("a", "a2"), ("b", "b1")] {
        send(&mut s, g, d);
    }
    let got = s.receive("q.fifo", 10);
    let bodies: Vec<_> = got.iter().map(|m| m["Body"].clone()).collect();
    assert_eq!(bodies, [json!("a1"), json!("b1")]);
    assert_eq!(got[0]["GroupId"], json!("a"));
    assert_eq!(got[1]["SequenceNumber"], json!(3));
    assert!(s.receive("q.fifo", 10).is_empty());
    s.run(json!({"Command": "DeleteMessage", "QueueName": "q.fifo", "ReceiptHandle": "r-1"}));
    let next = s.receive("q.fifo", 10);
    assert_eq!(next[0]["Body"], json!("a2"));
    assert_eq!(next.len(), 1);
}

#[test]
fn messages_beyond_the_receive_count_move_to_the_dead_letter_queue() {
    let mut s = Session::new();
    s.create("dlq", json!({"RetentionSeconds": 60}));
    s.create(
        "q",
        json!({"VisibilityTimeout": 0, "DeadLetterQueue": "dlq", "MaxReceiveCount": 2}),
    );
    s.send("q", "poison");
    s.send("q", "fine");
    assert_eq!(s.receive("q", 1)[0]["ReceiveCount"], json!(1));
    assert_eq!(s.receive("q", 1)[0]["ReceiveCount"], json!(2));
    // Third pass: poison moves out (not counted), "fine" is delivered.
    let third = s.receive("q", 1);
    assert_eq!(third.len(), 1);
    assert_eq!(third[0]["Body"], json!("fine"));
    assert_eq!(
        s.counts("dlq"),
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
    let moved = s.receive("dlq", 1);
    assert_eq!(moved[0]["MessageId"], json!("m-1"));
    assert_eq!(moved[0]["ReceiveCount"], json!(1));
    assert_eq!(moved[0]["FirstReceivedAt"], json!(0));
}

#[test]
fn dead_lettered_message_older_than_the_dlq_retention_is_dropped() {
    let mut s = Session::new();
    s.create("dlq", json!({"RetentionSeconds": 60}));
    s.create(
        "q",
        json!({"VisibilityTimeout": 0, "DeadLetterQueue": "dlq", "MaxReceiveCount": 1}),
    );
    s.send("q", "old");
    s.receive("q", 1);
    s.tick(60);
    assert!(s.receive("q", 1).is_empty());
    assert_eq!(
        s.counts("dlq"),
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn queue_limit_is_1000() {
    let mut s = Session::new();
    for i in 0..1000 {
        s.create(&format!("q{i}"), json!({}));
    }
    assert_eq!(
        s.code(json!({"Command": "CreateQueue", "QueueName": "extra"})),
        "LimitExceeded"
    );
    s.create("q5", json!({}));
}

#[test]
fn message_limit_is_100000_and_filling_is_fast() {
    let mut s = Session::new();
    s.create("q", json!({}));
    let line = br#"{"Command":"SendMessage","QueueName":"q","Body":"x"}"#;
    for _ in 0..100_000 {
        assert_eq!(s.raw(line)["ok"], json!(true));
    }
    assert_eq!(s.raw(line)["error"]["code"], json!("LimitExceeded"));
}
