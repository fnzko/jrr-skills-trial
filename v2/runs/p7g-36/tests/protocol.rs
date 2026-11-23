//! Behavior tests that drive the service through its line protocol.

use miniq::service::Service;
use serde_json::{Value, json};

struct Session(Service);

impl Session {
    fn new() -> Self {
        Session(Service::new())
    }

    fn raw(&mut self, line: &str) -> Value {
        serde_json::from_str(&self.0.handle_line(line.as_bytes())).unwrap()
    }

    fn call(&mut self, request: Value) -> Value {
        self.raw(&request.to_string())
    }

    /// Runs a command expected to succeed.
    fn ok(&mut self, request: Value) -> Value {
        let r = self.call(request);
        assert_eq!(r["ok"], true, "unexpected failure: {r}");
        r
    }

    /// Runs a command expected to fail and returns the error code.
    fn err(&mut self, request: Value) -> String {
        let r = self.call(request);
        assert_eq!(r["ok"], false, "unexpected success: {r}");
        r["error"]["code"].as_str().unwrap().to_owned()
    }

    fn create(&mut self, name: &str, attributes: Value) {
        self.ok(json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attributes}));
    }

    fn send(&mut self, queue: &str, body: &str) -> Value {
        self.ok(json!({"Command": "SendMessage", "QueueName": queue, "Body": body}))
    }

    fn receive(&mut self, queue: &str) -> Vec<Value> {
        self.receive_n(queue, 1)
    }

    fn receive_n(&mut self, queue: &str, n: u64) -> Vec<Value> {
        let r = self.ok(json!({"Command": "ReceiveMessage", "QueueName": queue, "MaxMessages": n}));
        r["Messages"].as_array().unwrap().clone()
    }

    fn tick(&mut self, seconds: u64) {
        self.ok(json!({"Command": "Tick", "Seconds": seconds}));
    }

    fn counts(&mut self, queue: &str) -> Value {
        self.ok(json!({"Command": "GetQueueAttributes", "QueueName": queue}))["Counts"].clone()
    }
}

#[test]
fn request_shape_errors() {
    let mut s = Session::new();
    for line in [
        "",
        "nope",
        "[]",
        "{}",
        r#"{"Command": 1}"#,
        r#"{"Command": "Nope"}"#,
        r#"{"command": "Tick"}"#,
        r#"{"Command":"Tick"} x"#,
    ] {
        assert_eq!(s.raw(line)["error"]["code"], "InvalidRequest", "{line}");
    }
}

#[test]
fn null_parameters_are_absent_and_unknown_members_ignored() {
    let mut s = Session::new();
    assert_eq!(
        s.err(json!({"Command": "Tick", "Seconds": null})),
        "MissingParameter"
    );
    s.ok(json!({"Command": "Tick", "Seconds": 1, "Extra": [1]}));
    s.ok(json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": {"Fifo": null, "DelaySeconds": null}}));
}

#[test]
fn check_order_missing_before_invalid_before_lookup() {
    let mut s = Session::new();
    assert_eq!(
        s.err(json!({"Command": "SendMessage", "QueueName": "ghost", "Body": 5})),
        "InvalidParameterValue"
    );
    assert_eq!(
        s.err(json!({"Command": "SendMessage", "QueueName": 5})),
        "MissingParameter"
    );
    assert_eq!(
        s.err(json!({"Command": "SendMessage", "QueueName": "ghost", "Body": "x"})),
        "QueueDoesNotExist"
    );
    assert_eq!(
        s.err(json!({"Command": "DeleteQueue", "QueueName": "bad name!"})),
        "QueueDoesNotExist"
    );
    assert_eq!(
        s.err(
            json!({"Command": "CreateQueue", "QueueName": "bad name", "Attributes": {"Nope": 1}})
        ),
        "QueueNameInvalid"
    );
    assert_eq!(
        s.err(json!({"Command": "CreateQueue", "QueueName": 7})),
        "InvalidParameterValue"
    );
    assert_eq!(
        s.err(json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": []})),
        "InvalidParameterValue"
    );
}

#[test]
fn integers_are_strict() {
    let mut s = Session::new();
    for text in ["30.0", "3e1", "\"30\"", "-1", "1209601"] {
        let r = s.raw(&format!(r#"{{"Command":"Tick","Seconds":{text}}}"#));
        assert_eq!(r["error"]["code"], "InvalidParameterValue", "{text}");
    }
}

#[test]
fn queue_names() {
    let mut s = Session::new();
    for name in [
        "",
        "a.b",
        ".fifo",
        "a b",
        "é",
        &"a".repeat(81),
        &format!("{}.fifo", "a".repeat(76)),
    ] {
        assert_eq!(
            s.err(json!({"Command": "CreateQueue", "QueueName": name})),
            "QueueNameInvalid",
            "{name}"
        );
    }
    s.create(&"a".repeat(80), json!({}));
    s.create(&format!("{}.fifo", "a".repeat(75)), json!({"Fifo": true}));
    assert_eq!(
        s.err(json!({"Command": "CreateQueue", "QueueName": "x.fifo"})),
        "QueueNameInvalid"
    );
    assert_eq!(
        s.err(json!({"Command": "CreateQueue", "QueueName": "x", "Attributes": {"Fifo": true}})),
        "QueueNameInvalid"
    );
}

#[test]
fn create_is_idempotent_only_for_equal_attributes() {
    let mut s = Session::new();
    s.create("q", json!({"DelaySeconds": 5}));
    s.create("q", json!({"DelaySeconds": 5}));
    assert_eq!(
        s.err(json!({"Command": "CreateQueue", "QueueName": "q"})),
        "QueueAlreadyExists"
    );
    s.create("dlq", json!({}));
    s.create(
        "withdlq",
        json!({"DeadLetterQueue": "dlq", "MaxReceiveCount": 2}),
    );
    s.create(
        "withdlq",
        json!({"DeadLetterQueue": "dlq", "MaxReceiveCount": 2}),
    );
    s.ok(json!({"Command": "DeleteQueue", "QueueName": "dlq"}));
    // Deleting the dead-letter queue unset the configuration on both attributes.
    s.create("withdlq", json!({}));
    let a = s.ok(json!({"Command": "GetQueueAttributes", "QueueName": "withdlq"}));
    assert_eq!(a["Attributes"].as_object().unwrap().len(), 5);
}

#[test]
fn create_rule_order() {
    let mut s = Session::new();
    let cbd = json!({"ContentBasedDeduplication": true});
    assert_eq!(
        s.err(json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": cbd})),
        "InvalidParameterValue"
    );
    assert_eq!(s.err(json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": {"MaxReceiveCount": 3}})), "InvalidParameterValue");
    assert_eq!(s.err(json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": {"DeadLetterQueue": "x", "MaxReceiveCount": 3}})), "QueueDoesNotExist");
    s.create("f.fifo", json!({"Fifo": true}));
    assert_eq!(s.err(json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": {"DeadLetterQueue": "f.fifo", "MaxReceiveCount": 3}})), "InvalidParameterValue");
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
    s.create("q5", json!({})); // idempotent create is not a new queue
}

#[test]
fn set_attributes() {
    let mut s = Session::new();
    s.create("q", json!({}));
    s.create("f.fifo", json!({"Fifo": true}));
    let set = |s: &mut Session, q: &str, a: Value| {
        s.call(json!({"Command": "SetQueueAttributes", "QueueName": q, "Attributes": a}))
    };
    assert_eq!(
        set(&mut s, "q", json!({"Fifo": false}))["error"]["code"],
        "UnsupportedOperation"
    );
    assert_eq!(
        set(&mut s, "q", json!({"ContentBasedDeduplication": true}))["error"]["code"],
        "InvalidParameterValue"
    );
    assert_eq!(
        set(
            &mut s,
            "q",
            json!({"DeadLetterQueue": "q", "MaxReceiveCount": 1})
        )["error"]["code"],
        "InvalidParameterValue"
    );
    assert_eq!(
        set(
            &mut s,
            "q",
            json!({"DeadLetterQueue": "f.fifo", "MaxReceiveCount": 1})
        )["error"]["code"],
        "InvalidParameterValue"
    );
    assert_eq!(
        set(
            &mut s,
            "q",
            json!({"DeadLetterQueue": "zz", "MaxReceiveCount": 1})
        )["error"]["code"],
        "QueueDoesNotExist"
    );
    assert_eq!(
        set(&mut s, "q", json!({"Bogus": 1}))["error"]["code"],
        "InvalidParameterValue"
    );
    assert_eq!(
        s.err(json!({"Command": "SetQueueAttributes", "QueueName": "q"})),
        "MissingParameter"
    );
    assert_eq!(set(&mut s, "q", json!({}))["ok"], true);
    s.create("d", json!({}));
    assert_eq!(
        set(
            &mut s,
            "q",
            json!({"VisibilityTimeout": 7, "DeadLetterQueue": "d", "MaxReceiveCount": 4})
        )["ok"],
        true
    );
    let a = s.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(
        a["Attributes"],
        json!({"VisibilityTimeout": 7, "DelaySeconds": 0, "RetentionSeconds": 345600, "Fifo": false, "ContentBasedDeduplication": false, "DeadLetterQueue": "d", "MaxReceiveCount": 4})
    );
    assert_eq!(
        a["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn list_queues_orders_by_bytes_and_filters_by_prefix() {
    let mut s = Session::new();
    s.create("b", json!({}));
    s.create("a_1", json!({}));
    s.create("a-1", json!({}));
    s.create("a1", json!({}));
    s.create("aB", json!({}));
    s.create("a.fifo", json!({"Fifo": true}));
    let all = s.ok(json!({"Command": "ListQueues"}));
    assert_eq!(
        all["QueueNames"],
        json!(["a-1", "a.fifo", "a1", "aB", "a_1", "b"])
    );
    let some = s.ok(json!({"Command": "ListQueues", "Prefix": "a"}));
    assert_eq!(some["QueueNames"].as_array().unwrap().len(), 5);
    assert_eq!(
        s.ok(json!({"Command": "ListQueues", "Prefix": "zz"}))["QueueNames"],
        json!([])
    );
}

#[test]
fn ids_and_basic_delivery() {
    let mut s = Session::new();
    s.create("q", json!({}));
    s.create("r", json!({}));
    assert_eq!(s.send("q", "one")["MessageId"], "m-1");
    assert_eq!(s.send("r", "two")["MessageId"], "m-2");
    assert_eq!(
        s.err(json!({"Command": "SendMessage", "QueueName": "q", "Body": ""})),
        "InvalidParameterValue"
    );
    assert_eq!(s.send("q", "three")["MessageId"], "m-3");
    s.tick(4);
    let got = s.receive_n("q", 10);
    assert_eq!(got.len(), 2);
    assert_eq!(got[0]["Body"], "one");
    assert_eq!(got[0]["ReceiptHandle"], "r-1");
    assert_eq!(got[1]["ReceiptHandle"], "r-2");
    assert_eq!(got[0]["SentAt"], 0);
    assert_eq!(got[0]["FirstReceivedAt"], 4);
    assert_eq!(got[0]["ReceiveCount"], 1);
    assert!(got[0].get("GroupId").is_none());
    assert_eq!(s.receive("q").len(), 0);
    assert_eq!(
        s.counts("q"),
        json!({"Visible": 0, "InFlight": 2, "Delayed": 0})
    );
    s.tick(30);
    let again = s.receive("q");
    assert_eq!(again[0]["Body"], "one");
    assert_eq!(again[0]["ReceiveCount"], 2);
    assert_eq!(again[0]["FirstReceivedAt"], 4);
    assert_eq!(again[0]["ReceiptHandle"], "r-3");
}

#[test]
fn body_size_is_measured_in_bytes() {
    let mut s = Session::new();
    s.create("q", json!({}));
    s.send("q", &"a".repeat(1024));
    assert_eq!(
        s.err(json!({"Command": "SendMessage", "QueueName": "q", "Body": "a".repeat(1025)})),
        "InvalidParameterValue"
    );
    s.send("q", &"é".repeat(512));
    assert_eq!(
        s.err(json!({"Command": "SendMessage", "QueueName": "q", "Body": "é".repeat(513)})),
        "InvalidParameterValue"
    );
}

#[test]
fn delay_and_unsupported_send_options() {
    let mut s = Session::new();
    s.create("q", json!({"DelaySeconds": 10}));
    s.send("q", "queue-delay");
    s.ok(json!({"Command": "SendMessage", "QueueName": "q", "Body": "now", "DelaySeconds": 0}));
    assert_eq!(
        s.counts("q"),
        json!({"Visible": 1, "InFlight": 0, "Delayed": 1})
    );
    s.tick(9);
    assert_eq!(s.receive_n("q", 5).len(), 1);
    s.tick(1);
    assert_eq!(s.receive_n("q", 5)[0]["Body"], "queue-delay");
    assert_eq!(
        s.err(json!({"Command": "SendMessage", "QueueName": "q", "Body": "x", "GroupId": "g"})),
        "UnsupportedOperation"
    );
    assert_eq!(
        s.err(
            json!({"Command": "SendMessage", "QueueName": "q", "Body": "x", "DeduplicationId": "d"})
        ),
        "UnsupportedOperation"
    );
    s.create("f.fifo", json!({"Fifo": true}));
    assert_eq!(s.err(json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "x", "DelaySeconds": 0, "GroupId": "g"})), "UnsupportedOperation");
}

#[test]
fn retention_removes_messages_whatever_their_state() {
    let mut s = Session::new();
    s.create("q", json!({"RetentionSeconds": 60}));
    s.send("q", "a");
    s.tick(59);
    s.send("q", "b");
    assert_eq!(s.receive("q").len(), 1); // a is now in flight
    s.tick(1);
    assert_eq!(
        s.counts("q"),
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
    s.tick(60);
    assert_eq!(
        s.counts("q"),
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn lowering_retention_acts_at_once_and_raising_does_not_restore() {
    let mut s = Session::new();
    s.create("q", json!({}));
    s.send("q", "a");
    s.tick(100);
    let set = |s: &mut Session, r: u64| {
        s.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"RetentionSeconds": r}}))
    };
    set(&mut s, 60);
    set(&mut s, 1_000);
    assert_eq!(
        s.counts("q"),
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn delete_and_change_visibility() {
    let mut s = Session::new();
    s.create("q", json!({}));
    s.create("other", json!({}));
    s.send("q", "a");
    let h = s.receive("q")[0]["ReceiptHandle"]
        .as_str()
        .unwrap()
        .to_owned();
    let delete = |s: &mut Session, q: &str, h: &str| {
        s.call(json!({"Command": "DeleteMessage", "QueueName": q, "ReceiptHandle": h}))
    };
    for bad in [
        "r-01",
        "R-1",
        "r-1 ",
        "r-2",
        "r-0",
        "x",
        "",
        "r-+1",
        "r-99999999999999999999999",
    ] {
        assert_eq!(
            delete(&mut s, "q", bad)["error"]["code"],
            "ReceiptHandleInvalid",
            "{bad}"
        );
    }
    assert_eq!(
        delete(&mut s, "other", &h)["error"]["code"],
        "ReceiptHandleExpired"
    );
    s.ok(json!({"Command": "ChangeMessageVisibility", "QueueName": "q", "ReceiptHandle": h, "VisibilityTimeout": 100}));
    s.tick(99);
    assert_eq!(s.receive("q").len(), 0);
    s.ok(json!({"Command": "ChangeMessageVisibility", "QueueName": "q", "ReceiptHandle": h, "VisibilityTimeout": 0}));
    assert_eq!(s.counts("q")["Visible"], 1);
    // The handle is not usable once the message is visible again.
    assert_eq!(
        delete(&mut s, "q", &h)["error"]["code"],
        "ReceiptHandleExpired"
    );
    let h2 = s.receive("q")[0]["ReceiptHandle"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        delete(&mut s, "q", &h)["error"]["code"],
        "ReceiptHandleExpired"
    );
    assert_eq!(delete(&mut s, "q", &h2)["ok"], true);
    assert_eq!(
        delete(&mut s, "q", &h2)["error"]["code"],
        "ReceiptHandleExpired"
    );
}

#[test]
fn zero_visibility_timeout_handle_is_never_usable() {
    let mut s = Session::new();
    s.create("q", json!({}));
    s.send("q", "a");
    let r = s.ok(json!({"Command": "ReceiveMessage", "QueueName": "q", "VisibilityTimeout": 0}));
    let h = r["Messages"][0]["ReceiptHandle"].clone();
    assert_eq!(
        s.err(json!({"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": h})),
        "ReceiptHandleExpired"
    );
}

#[test]
fn purge_and_recreate() {
    let mut s = Session::new();
    s.create("q", json!({}));
    s.send("q", "a");
    s.receive("q");
    s.send("q", "b");
    s.ok(json!({"Command": "PurgeQueue", "QueueName": "q"}));
    assert_eq!(
        s.counts("q"),
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
    s.ok(json!({"Command": "DeleteQueue", "QueueName": "q"}));
    assert_eq!(
        s.err(json!({"Command": "PurgeQueue", "QueueName": "q"})),
        "QueueDoesNotExist"
    );
}

#[test]
fn fifo_sequence_dedup_and_groups() {
    let mut s = Session::new();
    s.create("f.fifo", json!({"Fifo": true}));
    let send = |s: &mut Session, g: &str, d: &str| {
        s.call(json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": d, "GroupId": g, "DeduplicationId": d}))
    };
    assert_eq!(s.err(json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "x", "DeduplicationId": "d"})), "MissingParameter");
    assert_eq!(
        s.err(
            json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "x", "GroupId": "g"})
        ),
        "MissingParameter"
    );
    let a1 = send(&mut s, "a", "a1");
    assert_eq!(
        (a1["MessageId"].clone(), a1["SequenceNumber"].clone()),
        (json!("m-1"), json!(1))
    );
    send(&mut s, "a", "a2");
    send(&mut s, "b", "b1");
    // A duplicate uses up neither MessageId nor SequenceNumber.
    let dup = send(&mut s, "zzz", "a1");
    assert_eq!(
        (dup["MessageId"].clone(), dup["SequenceNumber"].clone()),
        (json!("m-1"), json!(1))
    );
    let got = s.receive_n("f.fifo", 10);
    let bodies: Vec<_> = got.iter().map(|m| m["Body"].as_str().unwrap()).collect();
    assert_eq!(bodies, ["a1", "b1"]);
    assert_eq!(got[0]["GroupId"], "a");
    assert_eq!(got[1]["SequenceNumber"], 3);
    // a1 is in flight: group a stays blocked even after b1 is deleted.
    assert_eq!(s.receive_n("f.fifo", 10).len(), 0);
    s.tick(30);
    assert_eq!(s.receive_n("f.fifo", 10).len(), 2);
}

#[test]
fn dedup_window_is_fixed_and_outlives_message() {
    let mut s = Session::new();
    s.create(
        "f.fifo",
        json!({"Fifo": true, "ContentBasedDeduplication": true}),
    );
    let send = |s: &mut Session| {
        s.ok(json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "same", "GroupId": "g"}))
    };
    assert_eq!(send(&mut s)["MessageId"], "m-1");
    s.ok(json!({"Command": "PurgeQueue", "QueueName": "f.fifo"}));
    s.tick(299);
    assert_eq!(send(&mut s)["MessageId"], "m-1"); // duplicate does not extend the window
    s.tick(1);
    let fresh = send(&mut s);
    assert_eq!(
        (fresh["MessageId"].clone(), fresh["SequenceNumber"].clone()),
        (json!("m-2"), json!(2))
    );
}

#[test]
fn dead_letter_moves_on_next_receive() {
    let mut s = Session::new();
    s.create("dlq", json!({}));
    s.create(
        "q",
        json!({"DeadLetterQueue": "dlq", "MaxReceiveCount": 2, "VisibilityTimeout": 1}),
    );
    s.send("q", "a");
    for _ in 0..2 {
        assert_eq!(s.receive("q").len(), 1);
        s.tick(1);
    }
    assert_eq!(s.counts("dlq")["Visible"], 0);
    assert_eq!(s.receive("q").len(), 0);
    assert_eq!(
        s.counts("q"),
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
    let moved = s.receive("dlq");
    assert_eq!(moved[0]["MessageId"], "m-1");
    assert_eq!(moved[0]["ReceiveCount"], 1);
    assert_eq!(moved[0]["SentAt"], 0);
}

#[test]
fn dead_lettered_messages_do_not_count_toward_max_messages() {
    let mut s = Session::new();
    s.create("dlq", json!({}));
    s.create(
        "q",
        json!({"DeadLetterQueue": "dlq", "MaxReceiveCount": 1, "VisibilityTimeout": 0}),
    );
    s.send("q", "old");
    assert_eq!(s.receive("q").len(), 1);
    s.send("q", "new");
    let got = s.receive("q");
    assert_eq!(got.len(), 1);
    assert_eq!(got[0]["Body"], "new");
    assert_eq!(s.receive("dlq")[0]["Body"], "old");
}

#[test]
fn fifo_dead_letter_unblocks_group_in_same_receive() {
    let mut s = Session::new();
    s.create("d.fifo", json!({"Fifo": true}));
    s.create("q.fifo", json!({"Fifo": true, "DeadLetterQueue": "d.fifo", "MaxReceiveCount": 1, "VisibilityTimeout": 0}));
    let send = |s: &mut Session, b: &str| {
        s.ok(json!({"Command": "SendMessage", "QueueName": "q.fifo", "Body": b, "GroupId": "g", "DeduplicationId": b}))
    };
    send(&mut s, "one");
    send(&mut s, "two");
    assert_eq!(s.receive("q.fifo")[0]["Body"], "one");
    let got = s.receive("q.fifo");
    assert_eq!(got.len(), 1);
    assert_eq!(got[0]["Body"], "two");
    let moved = s.receive("d.fifo");
    assert_eq!(moved[0]["Body"], "one");
    assert_eq!(moved[0]["SequenceNumber"], 1);
}

#[test]
fn dead_letter_arrival_respects_destination_retention() {
    let mut s = Session::new();
    s.create("dlq", json!({"RetentionSeconds": 60}));
    s.create(
        "q",
        json!({"DeadLetterQueue": "dlq", "MaxReceiveCount": 1, "VisibilityTimeout": 0}),
    );
    s.send("q", "a");
    s.receive("q");
    s.tick(60);
    assert_eq!(s.receive("q").len(), 0);
    assert_eq!(
        s.counts("dlq"),
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn delete_queue_unsets_dead_letter_configuration() {
    let mut s = Session::new();
    s.create("dlq", json!({}));
    s.create("q", json!({"DeadLetterQueue": "dlq", "MaxReceiveCount": 2}));
    s.ok(json!({"Command": "DeleteQueue", "QueueName": "dlq"}));
    let a = s.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert!(a["Attributes"].get("DeadLetterQueue").is_none());
    assert!(a["Attributes"].get("MaxReceiveCount").is_none());
}

#[test]
fn message_limit_is_enforced_quickly() {
    let mut s = Session::new();
    s.create("q", json!({}));
    let line = r#"{"Command":"SendMessage","QueueName":"q","Body":"x"}"#;
    for _ in 0..100_000 {
        s.0.handle_line(line.as_bytes());
    }
    assert_eq!(
        s.err(json!({"Command": "SendMessage", "QueueName": "q", "Body": "x"})),
        "LimitExceeded"
    );
    s.ok(json!({"Command": "PurgeQueue", "QueueName": "q"}));
    s.send("q", "x");
}

#[test]
fn draining_a_full_queue_one_message_at_a_time_is_fast() {
    let mut s = Session::new();
    s.create("q", json!({}));
    let send = r#"{"Command":"SendMessage","QueueName":"q","Body":"x"}"#;
    let receive = r#"{"Command":"ReceiveMessage","QueueName":"q"}"#;
    for _ in 0..50_000 {
        s.0.handle_line(send.as_bytes());
    }
    for _ in 0..50_000 {
        s.0.handle_line(receive.as_bytes());
    }
    assert_eq!(
        s.counts("q"),
        json!({"Visible": 0, "InFlight": 50_000, "Delayed": 0})
    );
}

#[test]
fn a_message_whose_visibility_ends_keeps_its_place() {
    let mut s = Session::new();
    s.create("q", json!({}));
    for body in ["a", "b", "c"] {
        s.send("q", body);
    }
    let first =
        s.ok(json!({"Command": "ReceiveMessage", "QueueName": "q", "VisibilityTimeout": 5}));
    assert_eq!(first["Messages"][0]["Body"], "a");
    assert_eq!(s.receive("q")[0]["Body"], "b");
    s.tick(5);
    let bodies: Vec<_> = s
        .receive_n("q", 10)
        .iter()
        .map(|m| m["Body"].clone())
        .collect();
    assert_eq!(bodies, [json!("a"), json!("c")]);
}

#[test]
fn a_delayed_message_does_not_hold_back_later_ones_in_a_standard_queue() {
    let mut s = Session::new();
    s.create("q", json!({}));
    s.ok(json!({"Command": "SendMessage", "QueueName": "q", "Body": "late", "DelaySeconds": 10}));
    s.send("q", "early");
    assert_eq!(s.receive("q")[0]["Body"], "early");
    s.tick(10);
    assert_eq!(s.receive("q")[0]["Body"], "late");
}
