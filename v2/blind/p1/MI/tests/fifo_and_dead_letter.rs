mod common;

use common::Session;
use serde_json::{Value, json};

fn fsend(q: &str, body: &str, group: &str, dedup: &str) -> Value {
    json!({"Command": "SendMessage", "QueueName": q, "Body": body, "GroupId": group, "DeduplicationId": dedup})
}

fn bodies(messages: &[Value]) -> Vec<&str> {
    messages
        .iter()
        .map(|m| m["Body"].as_str().unwrap())
        .collect()
}

#[test]
fn fifo_send_rules_in_order() {
    let mut s = Session::new();
    s.create("f.fifo", json!({"Fifo": true}));
    s.create(
        "c.fifo",
        json!({"Fifo": true, "ContentBasedDeduplication": true}),
    );
    let base = |q: &str| json!({"Command": "SendMessage", "QueueName": q, "Body": "x"});
    let with = |q: &str, extra: Value| {
        let mut v = base(q);
        v.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        v
    };
    assert_eq!(
        s.err(with("f.fifo", json!({"DelaySeconds": 1}))),
        "UnsupportedOperation"
    );
    // Unsupported wins over the missing GroupId.
    assert_eq!(s.err(base("f.fifo")), "MissingParameter");
    assert_eq!(
        s.err(with("f.fifo", json!({"GroupId": "g"}))),
        "MissingParameter"
    );
    let r = s.ok(with("c.fifo", json!({"GroupId": "g"})));
    assert_eq!(r["SequenceNumber"], 1);
}

#[test]
fn duplicates_return_the_original_and_use_no_numbers() {
    let mut s = Session::new();
    s.create("f.fifo", json!({"Fifo": true}));
    let first = s.ok(fsend("f.fifo", "a", "g", "d"));
    assert_eq!(first["MessageId"], "m-1");
    s.tick(100);
    // Same id, different body and group: still a duplicate.
    let dup = s.ok(fsend("f.fifo", "b", "other", "d"));
    assert_eq!(dup["MessageId"], "m-1");
    assert_eq!(dup["SequenceNumber"], 1);
    assert_eq!(s.counts("f.fifo"), [1, 0, 0]);
    // The duplicate did not extend the window: it ends 300 after the original.
    s.tick(199);
    assert_eq!(s.ok(fsend("f.fifo", "c", "g", "d"))["MessageId"], "m-1");
    s.tick(1);
    let again = s.ok(fsend("f.fifo", "c", "g", "d"));
    assert_eq!(again["MessageId"], "m-2");
    assert_eq!(again["SequenceNumber"], 2);
}

#[test]
fn the_dedup_id_outlives_the_message() {
    let mut s = Session::new();
    s.create("f.fifo", json!({"Fifo": true}));
    s.ok(fsend("f.fifo", "a", "g", "d"));
    let m = s.receive("f.fifo", 1);
    s.ok(json!({"Command": "DeleteMessage", "QueueName": "f.fifo", "ReceiptHandle": m[0]["ReceiptHandle"]}));
    assert_eq!(s.counts("f.fifo"), [0, 0, 0]);
    assert_eq!(s.ok(fsend("f.fifo", "a", "g", "d"))["MessageId"], "m-1");
    assert_eq!(s.counts("f.fifo"), [0, 0, 0]);
}

#[test]
fn content_based_dedup_uses_the_body_unless_an_id_is_given() {
    let mut s = Session::new();
    s.create(
        "c.fifo",
        json!({"Fifo": true, "ContentBasedDeduplication": true}),
    );
    let send = |body: &str, dedup: Option<&str>| {
        let mut v =
            json!({"Command": "SendMessage", "QueueName": "c.fifo", "Body": body, "GroupId": "g"});
        if let Some(d) = dedup {
            v["DeduplicationId"] = json!(d);
        }
        v
    };
    assert_eq!(s.ok(send("same", None))["MessageId"], "m-1");
    assert_eq!(s.ok(send("same", None))["MessageId"], "m-1");
    // An explicit id lives in the same namespace as bodies, by design of the spec.
    assert_eq!(s.ok(send("other", Some("same")))["MessageId"], "m-1");
    assert_eq!(s.ok(send("same", Some("x")))["MessageId"], "m-2");
}

#[test]
fn fifo_receive_gives_one_message_per_group_and_blocks_busy_groups() {
    let mut s = Session::new();
    s.create("f.fifo", json!({"Fifo": true}));
    for (body, group, id) in [
        ("a1", "A", "1"),
        ("a2", "A", "2"),
        ("b1", "B", "3"),
        ("c1", "C", "4"),
    ] {
        s.ok(fsend("f.fifo", body, group, id));
    }
    let got = s.receive("f.fifo", 10);
    assert_eq!(bodies(&got), ["a1", "b1", "c1"]);
    assert_eq!(got[0]["GroupId"], "A");
    assert_eq!(got[0]["SequenceNumber"], 1);
    // a1 is in flight, so A stays blocked; nothing else is visible.
    assert!(s.receive("f.fifo", 10).is_empty());
    s.ok(json!({"Command": "DeleteMessage", "QueueName": "f.fifo", "ReceiptHandle": got[0]["ReceiptHandle"]}));
    let got = s.receive("f.fifo", 10);
    assert_eq!(bodies(&got), ["a2"]);
}

#[test]
fn a_visible_message_behind_an_in_flight_one_of_its_group_waits() {
    let mut s = Session::new();
    s.create("f.fifo", json!({"Fifo": true, "VisibilityTimeout": 10}));
    s.ok(fsend("f.fifo", "a1", "A", "1"));
    s.receive("f.fifo", 1);
    s.ok(fsend("f.fifo", "a2", "A", "2"));
    s.ok(fsend("f.fifo", "b1", "B", "3"));
    assert_eq!(bodies(&s.receive("f.fifo", 10)), ["b1"]);
    s.tick(10);
    // a1 and b1 are visible again; a2 still waits behind a1.
    assert_eq!(bodies(&s.receive("f.fifo", 10)), ["a1", "b1"]);
}

fn dead_letter_setup(s: &mut Session, extra: Value) {
    s.create("dlq", extra);
    s.create(
        "src",
        json!({"VisibilityTimeout": 1, "DeadLetterQueue": "dlq", "MaxReceiveCount": 2}),
    );
}

#[test]
fn messages_move_to_the_dead_letter_queue_on_the_receive_after_the_limit() {
    let mut s = Session::new();
    dead_letter_setup(&mut s, json!({}));
    s.push("src", "poison");
    s.push("src", "fine");
    for _ in 0..2 {
        assert_eq!(bodies(&s.receive("src", 1)), ["poison"]);
        s.tick(1);
    }
    // The move does not use up MaxMessages: "fine" is delivered in the same call.
    let got = s.receive("src", 1);
    assert_eq!(bodies(&got), ["fine"]);
    assert_eq!(s.counts("src"), [0, 1, 0]);
    let moved = s.receive("dlq", 1);
    assert_eq!(moved[0]["MessageId"], "m-1");
    assert_eq!(moved[0]["Body"], "poison");
    assert_eq!(moved[0]["ReceiveCount"], 1);
    assert_eq!(moved[0]["SentAt"], 0);
    assert_eq!(moved[0]["FirstReceivedAt"], 2);
}

#[test]
fn a_moved_message_invalidates_its_old_handle() {
    let mut s = Session::new();
    dead_letter_setup(&mut s, json!({}));
    s.push("src", "poison");
    let mut last = Value::Null;
    for _ in 0..2 {
        last = s.receive("src", 1)[0]["ReceiptHandle"].clone();
        s.tick(1);
    }
    assert!(s.receive("src", 1).is_empty());
    assert_eq!(
        s.err(json!({"Command": "DeleteMessage", "QueueName": "src", "ReceiptHandle": last})),
        "ReceiptHandleExpired"
    );
    assert_eq!(s.counts("dlq"), [1, 0, 0]);
}

#[test]
fn dead_letter_arrival_respects_the_target_retention() {
    let mut s = Session::new();
    s.create("dlq", json!({"RetentionSeconds": 60}));
    s.create(
        "src",
        json!({"VisibilityTimeout": 100, "DeadLetterQueue": "dlq", "MaxReceiveCount": 1}),
    );
    s.push("src", "x");
    s.receive("src", 1);
    s.tick(100);
    s.receive("src", 1);
    assert_eq!(s.counts("src"), [0, 0, 0]);
    assert_eq!(s.counts("dlq"), [0, 0, 0]);
}

#[test]
fn fifo_dead_letters_get_new_sequence_numbers_and_unblock_their_group() {
    let mut s = Session::new();
    s.create("dlq.fifo", json!({"Fifo": true}));
    s.create(
        "src.fifo",
        json!({"Fifo": true, "VisibilityTimeout": 1, "DeadLetterQueue": "dlq.fifo", "MaxReceiveCount": 1}),
    );
    s.ok(fsend("dlq.fifo", "pre", "Z", "z"));
    s.ok(fsend("src.fifo", "a1", "A", "1"));
    s.ok(fsend("src.fifo", "a2", "A", "2"));
    assert_eq!(bodies(&s.receive("src.fifo", 5)), ["a1"]);
    s.tick(1);
    // a1 is dead-lettered, which frees group A for a2 within the same call.
    assert_eq!(bodies(&s.receive("src.fifo", 5)), ["a2"]);
    let moved = s.receive("dlq.fifo", 5);
    assert_eq!(bodies(&moved), ["pre", "a1"]);
    assert_eq!(moved[1]["SequenceNumber"], 2);
    assert_eq!(moved[1]["GroupId"], "A");
    assert_eq!(moved[1]["ReceiveCount"], 1);
}

#[test]
fn a_new_dead_letter_configuration_applies_from_the_next_receive() {
    let mut s = Session::new();
    s.create("dlq", json!({}));
    s.create("src", json!({"VisibilityTimeout": 0}));
    s.push("src", "x");
    for _ in 0..3 {
        s.receive("src", 1);
    }
    s.ok(json!({"Command": "SetQueueAttributes", "QueueName": "src", "Attributes": {"DeadLetterQueue": "dlq", "MaxReceiveCount": 3}}));
    assert_eq!(s.counts("dlq"), [0, 0, 0]);
    assert!(s.receive("src", 1).is_empty());
    assert_eq!(s.counts("dlq"), [1, 0, 0]);
}
