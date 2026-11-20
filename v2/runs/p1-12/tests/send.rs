mod common;

use common::Session;
use serde_json::{Value, json};

fn create(s: &mut Session, name: &str, attributes: Value) {
    let line = json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attributes});
    s.ok(&line.to_string());
}

fn send(s: &mut Session, queue: &str, body: &str) -> Value {
    s.ok(&json!({"Command": "SendMessage", "QueueName": queue, "Body": body}).to_string())
}

fn tick(s: &mut Session, seconds: u32) {
    s.ok(&format!(r#"{{"Command": "Tick", "Seconds": {seconds}}}"#));
}

fn counts(s: &mut Session, queue: &str) -> Value {
    let line = json!({"Command": "GetQueueAttributes", "QueueName": queue});
    s.ok(&line.to_string())["Counts"].clone()
}

fn fifo_send(s: &mut Session, queue: &str, group: &str, dedup: &str, body: &str) -> Value {
    let line = json!({
        "Command": "SendMessage", "QueueName": queue, "Body": body,
        "GroupId": group, "DeduplicationId": dedup,
    });
    s.ok(&line.to_string())
}

#[test]
fn standard_send_returns_global_message_ids() {
    let mut s = Session::new();
    create(&mut s, "a", json!({}));
    create(&mut s, "b", json!({}));
    assert_eq!(send(&mut s, "a", "x"), json!({"MessageId": "m-1"}));
    assert_eq!(send(&mut s, "b", "x"), json!({"MessageId": "m-2"}));
    // A failed send does not use up a number.
    assert_eq!(
        s.err(r#"{"Command": "SendMessage", "QueueName": "a", "Body": ""}"#),
        "InvalidParameterValue"
    );
    assert_eq!(send(&mut s, "a", "x"), json!({"MessageId": "m-3"}));
}

#[test]
fn get_queue_attributes_reports_attributes_and_counts() {
    let mut s = Session::new();
    create(&mut s, "dead", json!({}));
    create(
        &mut s,
        "q",
        json!({"VisibilityTimeout": 12, "DelaySeconds": 3, "RetentionSeconds": 100,
               "DeadLetterQueue": "dead", "MaxReceiveCount": 4}),
    );
    send(&mut s, "q", "x");
    assert_eq!(
        s.ok(r#"{"Command": "GetQueueAttributes", "QueueName": "q"}"#),
        json!({
            "Attributes": {
                "VisibilityTimeout": 12, "DelaySeconds": 3, "RetentionSeconds": 100,
                "Fifo": false, "ContentBasedDeduplication": false,
                "DeadLetterQueue": "dead", "MaxReceiveCount": 4,
            },
            "Counts": {"Visible": 0, "InFlight": 0, "Delayed": 1},
        })
    );
    assert_eq!(
        s.ok(r#"{"Command": "GetQueueAttributes", "QueueName": "dead"}"#),
        json!({
            "Attributes": {
                "VisibilityTimeout": 30, "DelaySeconds": 0, "RetentionSeconds": 345600,
                "Fifo": false, "ContentBasedDeduplication": false,
            },
            "Counts": {"Visible": 0, "InFlight": 0, "Delayed": 0},
        })
    );
    assert_eq!(
        s.err(r#"{"Command": "GetQueueAttributes", "QueueName": "nope"}"#),
        "QueueDoesNotExist"
    );
    assert_eq!(
        s.err(r#"{"Command": "GetQueueAttributes"}"#),
        "MissingParameter"
    );
}

#[test]
fn delay_comes_from_the_parameter_or_else_the_queue() {
    let mut s = Session::new();
    create(&mut s, "q", json!({"DelaySeconds": 10}));
    send(&mut s, "q", "queue default");
    s.ok(r#"{"Command": "SendMessage", "QueueName": "q", "Body": "x", "DelaySeconds": 0}"#);
    s.ok(r#"{"Command": "SendMessage", "QueueName": "q", "Body": "x", "DelaySeconds": 5}"#);
    assert_eq!(
        counts(&mut s, "q"),
        json!({"Visible": 1, "InFlight": 0, "Delayed": 2})
    );
    tick(&mut s, 4);
    assert_eq!(
        counts(&mut s, "q"),
        json!({"Visible": 1, "InFlight": 0, "Delayed": 2})
    );
    tick(&mut s, 1);
    assert_eq!(
        counts(&mut s, "q"),
        json!({"Visible": 2, "InFlight": 0, "Delayed": 1})
    );
    // A new queue delay applies to later sends only.
    s.ok(
        r#"{"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"DelaySeconds": 0}}"#,
    );
    assert_eq!(
        counts(&mut s, "q"),
        json!({"Visible": 2, "InFlight": 0, "Delayed": 1})
    );
    send(&mut s, "q", "undelayed");
    assert_eq!(
        counts(&mut s, "q"),
        json!({"Visible": 3, "InFlight": 0, "Delayed": 1})
    );
}

#[test]
fn body_length_is_counted_in_utf8_bytes() {
    let mut s = Session::new();
    create(&mut s, "q", json!({}));
    send(&mut s, "q", &"a".repeat(1024));
    send(&mut s, "q", &"é".repeat(512));
    for body in ["a".repeat(1025), "é".repeat(513), String::new()] {
        let line = json!({"Command": "SendMessage", "QueueName": "q", "Body": body});
        assert_eq!(s.err(&line.to_string()), "InvalidParameterValue");
    }
    assert_eq!(
        s.err(r#"{"Command": "SendMessage", "QueueName": "q", "Body": 5}"#),
        "InvalidParameterValue"
    );
    assert_eq!(
        s.err(r#"{"Command": "SendMessage", "QueueName": "q"}"#),
        "MissingParameter"
    );
    assert_eq!(
        s.err(r#"{"Command": "SendMessage", "Body": "x"}"#),
        "MissingParameter"
    );
}

#[test]
fn send_error_order() {
    let mut s = Session::new();
    create(&mut s, "std", json!({}));
    create(&mut s, "f.fifo", json!({"Fifo": true}));
    create(
        &mut s,
        "cb.fifo",
        json!({"Fifo": true, "ContentBasedDeduplication": true}),
    );
    // Values are checked before the queue lookup.
    assert_eq!(
        s.err(
            r#"{"Command": "SendMessage", "QueueName": "nope", "Body": "x", "DelaySeconds": 901}"#
        ),
        "InvalidParameterValue"
    );
    assert_eq!(
        s.err(r#"{"Command": "SendMessage", "QueueName": "nope", "Body": "x"}"#),
        "QueueDoesNotExist"
    );
    for (queue, extra) in [
        ("std", r#""GroupId": "g""#),
        ("std", r#""DeduplicationId": "d""#),
        (
            "f.fifo",
            r#""DelaySeconds": 0, "GroupId": "g", "DeduplicationId": "d""#,
        ),
    ] {
        let line = format!(
            r#"{{"Command": "SendMessage", "QueueName": "{queue}", "Body": "x", {extra}}}"#
        );
        assert_eq!(s.err(&line), "UnsupportedOperation", "{line}");
    }
    assert_eq!(
        s.err(r#"{"Command": "SendMessage", "QueueName": "f.fifo", "Body": "x", "DeduplicationId": "d"}"#),
        "MissingParameter"
    );
    assert_eq!(
        s.err(r#"{"Command": "SendMessage", "QueueName": "f.fifo", "Body": "x", "GroupId": "g"}"#),
        "MissingParameter"
    );
    s.ok(r#"{"Command": "SendMessage", "QueueName": "cb.fifo", "Body": "x", "GroupId": "g"}"#);
    for id in ["", "a b", "é", &"a".repeat(129)] {
        let line = json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "x",
                          "GroupId": id, "DeduplicationId": "d"});
        assert_eq!(
            s.err(&line.to_string()),
            "InvalidParameterValue",
            "group {id:?}"
        );
    }
    fifo_send(&mut s, "f.fifo", &"G".repeat(128), &"-_".repeat(64), "x");
}

#[test]
fn fifo_sequence_numbers_are_per_queue_and_survive_purge() {
    let mut s = Session::new();
    create(&mut s, "a.fifo", json!({"Fifo": true}));
    create(&mut s, "b.fifo", json!({"Fifo": true}));
    assert_eq!(
        fifo_send(&mut s, "a.fifo", "g", "1", "x"),
        json!({"MessageId": "m-1", "SequenceNumber": 1})
    );
    assert_eq!(
        fifo_send(&mut s, "b.fifo", "g", "1", "x"),
        json!({"MessageId": "m-2", "SequenceNumber": 1})
    );
    assert_eq!(
        fifo_send(&mut s, "a.fifo", "g", "2", "x"),
        json!({"MessageId": "m-3", "SequenceNumber": 2})
    );
    s.ok(r#"{"Command": "PurgeQueue", "QueueName": "a.fifo"}"#);
    assert_eq!(
        fifo_send(&mut s, "a.fifo", "g", "3", "x"),
        json!({"MessageId": "m-4", "SequenceNumber": 3})
    );
}

#[test]
fn duplicate_send_returns_the_original_and_uses_no_numbers() {
    let mut s = Session::new();
    create(&mut s, "f.fifo", json!({"Fifo": true}));
    assert_eq!(
        fifo_send(&mut s, "f.fifo", "g1", "d", "first"),
        json!({"MessageId": "m-1", "SequenceNumber": 1})
    );
    // Same id, other group and body: still a duplicate.
    assert_eq!(
        fifo_send(&mut s, "f.fifo", "g2", "d", "second"),
        json!({"MessageId": "m-1", "SequenceNumber": 1})
    );
    assert_eq!(
        fifo_send(&mut s, "f.fifo", "g1", "e", "third"),
        json!({"MessageId": "m-2", "SequenceNumber": 2})
    );
    assert_eq!(
        counts(&mut s, "f.fifo"),
        json!({"Visible": 2, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn deduplication_window_is_300_seconds_from_the_accepted_send() {
    let mut s = Session::new();
    create(&mut s, "f.fifo", json!({"Fifo": true}));
    fifo_send(&mut s, "f.fifo", "g", "d", "x");
    tick(&mut s, 299);
    assert_eq!(
        fifo_send(&mut s, "f.fifo", "g", "d", "x")["MessageId"],
        "m-1"
    );
    tick(&mut s, 1);
    // At t = 300 the id is free again, and the duplicate at 299 did not extend it.
    assert_eq!(
        fifo_send(&mut s, "f.fifo", "g", "d", "x")["MessageId"],
        "m-2"
    );
    tick(&mut s, 299);
    assert_eq!(
        fifo_send(&mut s, "f.fifo", "g", "d", "x")["MessageId"],
        "m-2"
    );
    tick(&mut s, 1);
    assert_eq!(
        fifo_send(&mut s, "f.fifo", "g", "d", "x")["MessageId"],
        "m-3"
    );
}

#[test]
fn deduplication_ids_outlive_the_message() {
    let mut s = Session::new();
    create(
        &mut s,
        "f.fifo",
        json!({"Fifo": true, "RetentionSeconds": 60}),
    );
    fifo_send(&mut s, "f.fifo", "g", "a", "x");
    fifo_send(&mut s, "f.fifo", "g", "b", "x");
    s.ok(r#"{"Command": "PurgeQueue", "QueueName": "f.fifo"}"#);
    assert_eq!(
        counts(&mut s, "f.fifo"),
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(
        fifo_send(&mut s, "f.fifo", "g", "a", "x")["MessageId"],
        "m-1"
    );
    tick(&mut s, 100);
    assert_eq!(
        counts(&mut s, "f.fifo"),
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(
        fifo_send(&mut s, "f.fifo", "g", "b", "x")["MessageId"],
        "m-2"
    );
}

#[test]
fn content_based_deduplication_uses_the_body_unless_an_id_is_given() {
    let mut s = Session::new();
    create(
        &mut s,
        "f.fifo",
        json!({"Fifo": true, "ContentBasedDeduplication": true}),
    );
    let by_body = |s: &mut Session, body: &str| {
        let line =
            json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": body, "GroupId": "g"});
        s.ok(&line.to_string())["MessageId"].clone()
    };
    assert_eq!(by_body(&mut s, "same"), "m-1");
    assert_eq!(by_body(&mut s, "same"), "m-1");
    assert_eq!(by_body(&mut s, "other"), "m-2");
    // An explicit id is separate from the body it happens to equal.
    assert_eq!(
        fifo_send(&mut s, "f.fifo", "g", "same", "different body")["MessageId"],
        "m-1"
    );
    assert_eq!(
        fifo_send(&mut s, "f.fifo", "g", "fresh", "same")["MessageId"],
        "m-3"
    );
}

#[test]
fn retention_removes_messages_at_the_deadline() {
    let mut s = Session::new();
    create(
        &mut s,
        "q",
        json!({"RetentionSeconds": 60, "DelaySeconds": 900}),
    );
    send(&mut s, "q", "x");
    tick(&mut s, 59);
    assert_eq!(
        counts(&mut s, "q"),
        json!({"Visible": 0, "InFlight": 0, "Delayed": 1})
    );
    tick(&mut s, 1);
    assert_eq!(
        counts(&mut s, "q"),
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn lowering_retention_removes_at_once_and_raising_it_revives_nothing() {
    let mut s = Session::new();
    create(&mut s, "q", json!({"RetentionSeconds": 1000}));
    send(&mut s, "q", "old");
    tick(&mut s, 100);
    send(&mut s, "q", "new");
    s.ok(r#"{"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"RetentionSeconds": 60}}"#);
    assert_eq!(
        counts(&mut s, "q"),
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );

    create(&mut s, "r", json!({"RetentionSeconds": 60}));
    send(&mut s, "r", "x");
    tick(&mut s, 61);
    // Nothing looked at r while its message expired.
    s.ok(r#"{"Command": "SetQueueAttributes", "QueueName": "r", "Attributes": {"RetentionSeconds": 1000}}"#);
    assert_eq!(
        counts(&mut s, "r"),
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn purge_empties_the_queue() {
    let mut s = Session::new();
    create(&mut s, "q", json!({"DelaySeconds": 5}));
    send(&mut s, "q", "a");
    tick(&mut s, 5);
    send(&mut s, "q", "b");
    assert_eq!(
        counts(&mut s, "q"),
        json!({"Visible": 1, "InFlight": 0, "Delayed": 1})
    );
    assert_eq!(
        s.send(r#"{"Command": "PurgeQueue", "QueueName": "q"}"#),
        json!({"ok": true})
    );
    assert_eq!(
        counts(&mut s, "q"),
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(
        s.err(r#"{"Command": "PurgeQueue", "QueueName": "nope"}"#),
        "QueueDoesNotExist"
    );
}

#[test]
fn a_queue_holds_at_most_100000_messages() {
    let mut s = Session::new();
    create(&mut s, "q", json!({}));
    let line = r#"{"Command": "SendMessage", "QueueName": "q", "Body": "x"}"#;
    for _ in 0..100_000 {
        assert_eq!(s.send(line)["ok"], true);
    }
    assert_eq!(s.err(line), "LimitExceeded");
    s.ok(r#"{"Command": "PurgeQueue", "QueueName": "q"}"#);
    assert_eq!(s.ok(line)["MessageId"], "m-100001");
}

#[test]
fn a_recreated_queue_shares_nothing_with_the_deleted_one() {
    let mut s = Session::new();
    create(&mut s, "f.fifo", json!({"Fifo": true}));
    fifo_send(&mut s, "f.fifo", "g", "d", "x");
    s.ok(r#"{"Command": "DeleteQueue", "QueueName": "f.fifo"}"#);
    create(&mut s, "f.fifo", json!({"Fifo": true}));
    assert_eq!(
        fifo_send(&mut s, "f.fifo", "g", "d", "x"),
        json!({"MessageId": "m-2", "SequenceNumber": 1})
    );
}
