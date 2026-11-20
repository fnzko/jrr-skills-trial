mod common;

use common::Session;
use serde_json::{Value, json};

fn send(queue: &str, body: &str) -> Value {
    json!({"Command": "SendMessage", "QueueName": queue, "Body": body})
}

fn receive(queue: &str) -> Value {
    json!({"Command": "ReceiveMessage", "QueueName": queue})
}

fn handle(m: &Value) -> String {
    m["ReceiptHandle"].as_str().unwrap().to_owned()
}

#[test]
fn ids_count_accepted_messages_over_all_queues() {
    let mut s = Session::new();
    s.create("a", json!({}));
    s.create("b", json!({}));
    assert_eq!(s.push("a", "x"), "m-1");
    assert_eq!(s.push("b", "x"), "m-2");
    assert_eq!(
        s.err(json!({"Command": "SendMessage", "QueueName": "a", "Body": ""})),
        "InvalidParameterValue"
    );
    assert_eq!(s.push("a", "x"), "m-3");
}

#[test]
fn standard_send_rules() {
    let mut s = Session::new();
    s.create("a", json!({}));
    let with =
        |k: &str, v: Value| json!({"Command": "SendMessage", "QueueName": "a", "Body": "x", k: v});
    assert_eq!(s.err(with("GroupId", json!("g"))), "UnsupportedOperation");
    assert_eq!(
        s.err(with("DeduplicationId", json!("d"))),
        "UnsupportedOperation"
    );
    // A null parameter is absent.
    s.ok(with("GroupId", Value::Null));
    let r = s.ok(with("DelaySeconds", json!(10)));
    assert!(r.get("SequenceNumber").is_none());
    assert_eq!(s.counts("a"), [1, 0, 1]);
}

#[test]
fn delay_defaults_to_the_queue_value_at_send_time() {
    let mut s = Session::new();
    s.create("a", json!({"DelaySeconds": 10}));
    s.push("a", "one");
    s.ok(json!({"Command": "SetQueueAttributes", "QueueName": "a", "Attributes": {"DelaySeconds": 0}}));
    s.push("a", "two");
    s.ok(json!({"Command": "SendMessage", "QueueName": "a", "Body": "three", "DelaySeconds": 3}));
    assert_eq!(s.counts("a"), [1, 0, 2]);
    s.tick(3);
    assert_eq!(s.counts("a"), [2, 0, 1]);
    s.tick(7);
    assert_eq!(s.counts("a"), [3, 0, 0]);
}

#[test]
fn receive_delivers_in_arrival_order_and_marks_in_flight() {
    let mut s = Session::new();
    s.create("a", json!({}));
    for body in ["1", "2", "3"] {
        s.push("a", body);
    }
    s.tick(4);
    let got = s.receive("a", 2);
    assert_eq!(got.len(), 2);
    assert_eq!(got[0]["Body"], "1");
    assert_eq!(got[1]["Body"], "2");
    assert_eq!(got[0]["ReceiptHandle"], "r-1");
    assert_eq!(got[1]["ReceiptHandle"], "r-2");
    assert_eq!(got[0]["ReceiveCount"], 1);
    assert_eq!(got[0]["SentAt"], 0);
    assert_eq!(got[0]["FirstReceivedAt"], 4);
    assert!(got[0].get("GroupId").is_none());
    assert_eq!(s.counts("a"), [1, 2, 0]);

    // After the timeout the message is back in its original place.
    s.tick(30);
    let got = s.receive("a", 10);
    let bodies: Vec<_> = got.iter().map(|m| m["Body"].as_str().unwrap()).collect();
    assert_eq!(bodies, ["1", "2", "3"]);
    assert_eq!(got[0]["ReceiveCount"], 2);
    assert_eq!(got[0]["FirstReceivedAt"], 4);
    assert_eq!(got[2]["ReceiveCount"], 1);
}

#[test]
fn receive_parameter_checks() {
    let mut s = Session::new();
    s.create("a", json!({}));
    for bad in [json!(0), json!(11), json!("1"), json!(1.0)] {
        assert_eq!(
            s.err(json!({"Command": "ReceiveMessage", "QueueName": "a", "MaxMessages": bad})),
            "InvalidParameterValue"
        );
    }
    assert_eq!(
        s.err(json!({"Command": "ReceiveMessage", "QueueName": "a", "VisibilityTimeout": 43201})),
        "InvalidParameterValue"
    );
    assert_eq!(s.ok(receive("a"))["Messages"], json!([]));
}

#[test]
fn zero_visibility_timeout_leaves_the_message_visible_with_a_dead_handle() {
    let mut s = Session::new();
    s.create("a", json!({}));
    s.push("a", "x");
    let r = s.ok(json!({"Command": "ReceiveMessage", "QueueName": "a", "VisibilityTimeout": 0}));
    let h = handle(&r["Messages"][0]);
    assert_eq!(s.counts("a"), [1, 0, 0]);
    assert_eq!(
        s.err(json!({"Command": "DeleteMessage", "QueueName": "a", "ReceiptHandle": h})),
        "ReceiptHandleExpired"
    );
}

#[test]
fn receipt_handle_errors() {
    let mut s = Session::new();
    s.create("a", json!({}));
    s.create("b", json!({}));
    s.push("a", "x");
    let first = handle(&s.receive("a", 1)[0]);
    let del =
        |q: &str, h: &str| json!({"Command": "DeleteMessage", "QueueName": q, "ReceiptHandle": h});
    for never in [
        "r-01",
        "R-1",
        "r-1 ",
        "r-2",
        "r-0",
        "r-",
        "1",
        "r--1",
        "r-99999999999999999999999",
    ] {
        assert_eq!(s.err(del("a", never)), "ReceiptHandleInvalid", "{never}");
    }
    // Issued, but for a message of another queue.
    assert_eq!(s.err(del("b", &first)), "ReceiptHandleExpired");
    // Redelivered since: only the latest handle works.
    s.tick(30);
    let second = handle(&s.receive("a", 1)[0]);
    assert_eq!(s.err(del("a", &first)), "ReceiptHandleExpired");
    s.ok(del("a", &second));
    assert_eq!(s.err(del("a", &second)), "ReceiptHandleExpired");
    assert_eq!(s.counts("a"), [0, 0, 0]);
    // Handle errors come after the queue lookup.
    assert_eq!(s.err(del("zz", "r-1")), "QueueDoesNotExist");
}

#[test]
fn change_visibility() {
    let mut s = Session::new();
    s.create("a", json!({}));
    s.push("a", "x");
    let h = handle(&s.receive("a", 1)[0]);
    let change = |t: u32| json!({"Command": "ChangeMessageVisibility", "QueueName": "a", "ReceiptHandle": h, "VisibilityTimeout": t});
    s.tick(10);
    s.ok(change(100));
    s.tick(99);
    assert_eq!(s.counts("a"), [0, 1, 0]);
    s.tick(1);
    assert_eq!(s.counts("a"), [1, 0, 0]);
    assert_eq!(s.err(change(5)), "ReceiptHandleExpired");

    let h2 = handle(&s.receive("a", 1)[0]);
    assert_eq!(h2, "r-2");
    s.ok(json!({"Command": "ChangeMessageVisibility", "QueueName": "a", "ReceiptHandle": h2, "VisibilityTimeout": 0}));
    assert_eq!(s.counts("a"), [1, 0, 0]);
    assert_eq!(s.receive("a", 1)[0]["ReceiveCount"], 3);
    assert_eq!(
        s.err(
            json!({"Command": "ChangeMessageVisibility", "QueueName": "a", "ReceiptHandle": "r-3"})
        ),
        "MissingParameter"
    );
    assert_eq!(
        s.err(json!({"Command": "ChangeMessageVisibility", "QueueName": "a", "ReceiptHandle": "r-3", "VisibilityTimeout": 43201})),
        "InvalidParameterValue"
    );
}

#[test]
fn retention_removes_messages_in_any_state_and_is_not_undone() {
    let mut s = Session::new();
    s.create("a", json!({"RetentionSeconds": 100}));
    s.push("a", "inflight");
    s.push("a", "visible");
    s.ok(json!({"Command": "ReceiveMessage", "QueueName": "a", "VisibilityTimeout": 1000}));
    s.ok(
        json!({"Command": "SendMessage", "QueueName": "a", "Body": "delayed", "DelaySeconds": 500}),
    );
    s.tick(99);
    assert_eq!(s.counts("a"), [1, 1, 1]);
    s.tick(1);
    assert_eq!(s.counts("a"), [0, 0, 0]);

    // Lowering retention removes at once; raising it later does not restore.
    s.create("b", json!({}));
    s.push("b", "old");
    s.tick(60);
    s.push("b", "new");
    s.ok(json!({"Command": "SetQueueAttributes", "QueueName": "b", "Attributes": {"RetentionSeconds": 60}}));
    assert_eq!(s.counts("b"), [1, 0, 0]);
    s.ok(json!({"Command": "SetQueueAttributes", "QueueName": "b", "Attributes": {"RetentionSeconds": 1000}}));
    assert_eq!(s.counts("b"), [1, 0, 0]);
}

#[test]
fn purge_removes_everything_and_keeps_counters() {
    let mut s = Session::new();
    s.create("f.fifo", json!({"Fifo": true}));
    let f = |body: &str, dedup: &str| json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": body, "GroupId": "g", "DeduplicationId": dedup});
    assert_eq!(s.ok(f("a", "d1"))["SequenceNumber"], 1);
    let h = handle(&s.receive("f.fifo", 1)[0]);
    s.ok(json!({"Command": "PurgeQueue", "QueueName": "f.fifo"}));
    assert_eq!(s.counts("f.fifo"), [0, 0, 0]);
    assert_eq!(
        s.err(json!({"Command": "DeleteMessage", "QueueName": "f.fifo", "ReceiptHandle": h})),
        "ReceiptHandleExpired"
    );
    // The deduplication window survives the purge; sequence numbers go on.
    assert_eq!(s.ok(f("a", "d1"))["SequenceNumber"], 1);
    assert_eq!(s.counts("f.fifo"), [0, 0, 0]);
    assert_eq!(s.ok(f("a", "d2"))["SequenceNumber"], 2);
}

#[test]
fn message_limit_is_100000_and_filling_is_fast() {
    let mut s = Session::new();
    s.create("a", json!({}));
    let line = send("a", "x").to_string();
    for _ in 0..100_000 {
        let r = s.raw(&line);
        assert_eq!(r["ok"], true);
    }
    assert_eq!(s.err(send("a", "x")), "LimitExceeded");
    s.ok(json!({"Command": "PurgeQueue", "QueueName": "a"}));
    s.push("a", "x");
}

#[test]
fn the_message_limit_counts_every_state() {
    let mut s = Session::new();
    s.create("a", json!({"DelaySeconds": 900}));
    let line = send("a", "x").to_string();
    for _ in 0..100_000 {
        s.raw(&line);
    }
    assert_eq!(s.counts("a"), [0, 0, 100_000]);
    assert_eq!(s.err(send("a", "x")), "LimitExceeded");
}
