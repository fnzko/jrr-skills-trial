mod common;

use common::Miniq;
use serde_json::{Value, json};

fn create(miniq: &mut Miniq, name: &str, attributes: Value) {
    miniq.ok(json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attributes}));
}

fn fifo(miniq: &mut Miniq, name: &str) {
    create(miniq, name, json!({"Fifo": true}));
}

fn send(queue: &str, body: &str) -> Value {
    json!({"Command": "SendMessage", "QueueName": queue, "Body": body})
}

fn send_fifo(queue: &str, body: &str, group: &str, dedup: &str) -> Value {
    json!({"Command": "SendMessage", "QueueName": queue, "Body": body, "GroupId": group, "DeduplicationId": dedup})
}

fn tick(miniq: &mut Miniq, seconds: u64) {
    miniq.ok(json!({"Command": "Tick", "Seconds": seconds}));
}

fn counts(miniq: &mut Miniq, queue: &str) -> Value {
    miniq.ok(json!({"Command": "GetQueueAttributes", "QueueName": queue}))["Counts"].clone()
}

#[test]
fn message_ids_count_accepted_messages_over_all_queues() {
    let mut miniq = Miniq::new();
    create(&mut miniq, "a", json!({}));
    create(&mut miniq, "b", json!({}));
    assert_eq!(
        miniq.ok(send("a", "x")),
        json!({"ok": true, "MessageId": "m-1"})
    );
    assert_eq!(miniq.ok(send("b", "x"))["MessageId"], "m-2");
    miniq.fails(send("missing", "x"), "QueueDoesNotExist");
    miniq.fails(send("a", ""), "InvalidParameterValue");
    assert_eq!(miniq.ok(send("a", "x"))["MessageId"], "m-3");
    assert_eq!(
        counts(&mut miniq, "a"),
        json!({"Visible": 2, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn body_is_limited_in_utf8_bytes() {
    let mut miniq = Miniq::new();
    create(&mut miniq, "q", json!({}));
    miniq.ok(send("q", &"a".repeat(1024)));
    miniq.ok(send("q", &"\u{e9}".repeat(512)));
    miniq.ok(send("q", &"\u{1F600}".repeat(256)));
    miniq.fails(send("q", &"a".repeat(1025)), "InvalidParameterValue");
    miniq.fails(
        send("q", &format!("{}a", "\u{e9}".repeat(512))),
        "InvalidParameterValue",
    );
    miniq.fails(
        json!({"Command": "SendMessage", "QueueName": "q", "Body": 5}),
        "InvalidParameterValue",
    );
}

#[test]
fn delay_comes_from_the_message_or_else_the_queue() {
    let mut miniq = Miniq::new();
    create(&mut miniq, "q", json!({"DelaySeconds": 10}));
    miniq.ok(send("q", "queue default"));
    miniq.ok(json!({"Command": "SendMessage", "QueueName": "q", "Body": "own", "DelaySeconds": 2}));
    miniq
        .ok(json!({"Command": "SendMessage", "QueueName": "q", "Body": "none", "DelaySeconds": 0}));
    assert_eq!(
        counts(&mut miniq, "q"),
        json!({"Visible": 1, "InFlight": 0, "Delayed": 2})
    );
    tick(&mut miniq, 1);
    assert_eq!(
        counts(&mut miniq, "q"),
        json!({"Visible": 1, "InFlight": 0, "Delayed": 2})
    );
    tick(&mut miniq, 1);
    assert_eq!(
        counts(&mut miniq, "q"),
        json!({"Visible": 2, "InFlight": 0, "Delayed": 1})
    );
    tick(&mut miniq, 8);
    assert_eq!(
        counts(&mut miniq, "q"),
        json!({"Visible": 3, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn queue_delay_change_applies_to_later_sends_only() {
    let mut miniq = Miniq::new();
    create(&mut miniq, "q", json!({"DelaySeconds": 10}));
    miniq.ok(send("q", "old"));
    miniq.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"DelaySeconds": 0}}));
    miniq.ok(send("q", "new"));
    assert_eq!(
        counts(&mut miniq, "q"),
        json!({"Visible": 1, "InFlight": 0, "Delayed": 1})
    );
}

#[test]
fn send_parameter_errors_follow_the_documented_order() {
    let mut miniq = Miniq::new();
    create(&mut miniq, "q", json!({}));
    miniq.fails(
        json!({"Command": "SendMessage", "Body": "x", "DelaySeconds": 901}),
        "MissingParameter",
    );
    miniq.fails(
        json!({"Command": "SendMessage", "QueueName": "q", "DelaySeconds": 901}),
        "MissingParameter",
    );
    miniq.fails(
        json!({"Command": "SendMessage", "QueueName": "nope", "Body": "x", "DelaySeconds": 901}),
        "InvalidParameterValue",
    );
    miniq.fails(
        json!({"Command": "SendMessage", "QueueName": "nope", "Body": "x", "GroupId": "g"}),
        "QueueDoesNotExist",
    );
    for delay in [json!(-1), json!(901), json!(1.5), json!("1")] {
        miniq.fails(
            json!({"Command": "SendMessage", "QueueName": "q", "Body": "x", "DelaySeconds": delay}),
            "InvalidParameterValue",
        );
    }
    let too_long = "g".repeat(129);
    for id in ["", "a b", "a.b", "\u{e9}", too_long.as_str()] {
        for member in ["GroupId", "DeduplicationId"] {
            miniq.fails(
                json!({"Command": "SendMessage", "QueueName": "q", "Body": "x", member: id}),
                "InvalidParameterValue",
            );
        }
    }
    // A bad id outranks the unsupported-operation rule.
    miniq.fails(
        json!({"Command": "SendMessage", "QueueName": "q", "Body": "x", "GroupId": 1}),
        "InvalidParameterValue",
    );
    miniq.ok(json!({"Command": "SendMessage", "QueueName": "q", "Body": "x", "DelaySeconds": null, "GroupId": null, "DeduplicationId": null}));
}

#[test]
fn unsupported_and_missing_parameters_depend_on_the_queue_type() {
    let mut miniq = Miniq::new();
    create(&mut miniq, "std", json!({}));
    fifo(&mut miniq, "f.fifo");
    create(
        &mut miniq,
        "cbd.fifo",
        json!({"Fifo": true, "ContentBasedDeduplication": true}),
    );
    let with = |queue: &str, member: &str, value: Value| json!({"Command": "SendMessage", "QueueName": queue, "Body": "x", member: value});
    miniq.fails(with("std", "GroupId", json!("g")), "UnsupportedOperation");
    miniq.fails(
        with("std", "DeduplicationId", json!("d")),
        "UnsupportedOperation",
    );
    miniq.fails(
        with("f.fifo", "DelaySeconds", json!(0)),
        "UnsupportedOperation",
    );
    // Unsupported outranks missing.
    miniq.fails(
        with("f.fifo", "DelaySeconds", json!(5)),
        "UnsupportedOperation",
    );
    miniq.fails(send("f.fifo", "x"), "MissingParameter");
    miniq.fails(
        with("f.fifo", "DeduplicationId", json!("d")),
        "MissingParameter",
    );
    miniq.fails(with("f.fifo", "GroupId", json!("g")), "MissingParameter");
    miniq.ok(with("cbd.fifo", "GroupId", json!("g")));
}

#[test]
fn fifo_sequence_numbers_count_per_queue_and_duplicates_return_the_original() {
    let mut miniq = Miniq::new();
    fifo(&mut miniq, "a.fifo");
    fifo(&mut miniq, "b.fifo");
    assert_eq!(
        miniq.ok(send_fifo("a.fifo", "x", "g", "d1")),
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
    assert_eq!(
        miniq.ok(send_fifo("a.fifo", "y", "h", "d2")),
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2})
    );
    assert_eq!(
        miniq.ok(send_fifo("b.fifo", "x", "g", "d1")),
        json!({"ok": true, "MessageId": "m-3", "SequenceNumber": 1})
    );
    // Same id, other body and group: still a duplicate, and no id is used.
    assert_eq!(
        miniq.ok(send_fifo("a.fifo", "other", "z", "d1")),
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
    assert_eq!(
        miniq.ok(send_fifo("a.fifo", "z", "g", "d3")),
        json!({"ok": true, "MessageId": "m-4", "SequenceNumber": 3})
    );
    assert_eq!(counts(&mut miniq, "a.fifo")["Visible"], 3);
}

#[test]
fn deduplication_window_is_300_seconds_from_the_accepted_send() {
    let mut miniq = Miniq::new();
    fifo(&mut miniq, "q.fifo");
    miniq.ok(send_fifo("q.fifo", "x", "g", "d"));
    tick(&mut miniq, 200);
    assert_eq!(
        miniq.ok(send_fifo("q.fifo", "x", "g", "d"))["MessageId"],
        "m-1"
    );
    tick(&mut miniq, 99);
    assert_eq!(
        miniq.ok(send_fifo("q.fifo", "x", "g", "d"))["MessageId"],
        "m-1"
    );
    tick(&mut miniq, 1);
    // The duplicate at 200 did not extend the window.
    let reopened = miniq.ok(send_fifo("q.fifo", "x", "g", "d"));
    assert_eq!(
        reopened,
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2})
    );
    tick(&mut miniq, 299);
    assert_eq!(
        miniq.ok(send_fifo("q.fifo", "x", "g", "d"))["MessageId"],
        "m-2"
    );
    tick(&mut miniq, 1);
    assert_eq!(
        miniq.ok(send_fifo("q.fifo", "x", "g", "d"))["MessageId"],
        "m-3"
    );
}

#[test]
fn content_based_deduplication_uses_the_body_unless_an_id_is_given() {
    let mut miniq = Miniq::new();
    create(
        &mut miniq,
        "q.fifo",
        json!({"Fifo": true, "ContentBasedDeduplication": true}),
    );
    let body_only = |body: &str, group: &str| json!({"Command": "SendMessage", "QueueName": "q.fifo", "Body": body, "GroupId": group});
    assert_eq!(miniq.ok(body_only("same", "g"))["MessageId"], "m-1");
    assert_eq!(miniq.ok(body_only("same", "h"))["MessageId"], "m-1");
    assert_eq!(miniq.ok(body_only("different", "g"))["MessageId"], "m-2");
    // An explicit id is a different key than the body, even if equal to it.
    assert_eq!(
        miniq.ok(send_fifo("q.fifo", "same", "g", "other"))["MessageId"],
        "m-3"
    );
    assert_eq!(
        miniq.ok(send_fifo("q.fifo", "zzz", "g", "same"))["MessageId"],
        "m-1"
    );
}

#[test]
fn purge_removes_messages_but_keeps_sequence_numbers_and_deduplication() {
    let mut miniq = Miniq::new();
    fifo(&mut miniq, "q.fifo");
    miniq.ok(send_fifo("q.fifo", "x", "g", "d1"));
    miniq.ok(send_fifo("q.fifo", "y", "g", "d2"));
    miniq.ok(json!({"Command": "PurgeQueue", "QueueName": "q.fifo"}));
    assert_eq!(
        counts(&mut miniq, "q.fifo"),
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(
        miniq.ok(send_fifo("q.fifo", "x", "g", "d1")),
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
    assert_eq!(
        miniq.ok(send_fifo("q.fifo", "x", "g", "d3")),
        json!({"ok": true, "MessageId": "m-3", "SequenceNumber": 3})
    );
    assert_eq!(counts(&mut miniq, "q.fifo")["Visible"], 1);
}

#[test]
fn deleted_and_recreated_queue_shares_nothing() {
    let mut miniq = Miniq::new();
    fifo(&mut miniq, "q.fifo");
    miniq.ok(send_fifo("q.fifo", "x", "g", "d"));
    miniq.ok(json!({"Command": "DeleteQueue", "QueueName": "q.fifo"}));
    fifo(&mut miniq, "q.fifo");
    assert_eq!(
        miniq.ok(send_fifo("q.fifo", "x", "g", "d")),
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 1})
    );
}

#[test]
fn a_queue_holds_at_most_100000_messages_and_fills_quickly() {
    let mut miniq = Miniq::new();
    create(&mut miniq, "q", json!({}));
    for _ in 0..100_000 {
        let response = miniq.call(send("q", "x"));
        assert_eq!(response["ok"], true);
    }
    miniq.fails(send("q", "x"), "LimitExceeded");
    assert_eq!(counts(&mut miniq, "q")["Visible"], 100_000);
    // The failed send used no id.
    miniq.ok(json!({"Command": "PurgeQueue", "QueueName": "q"}));
    assert_eq!(miniq.ok(send("q", "x"))["MessageId"], "m-100001");
}

#[test]
fn a_duplicate_is_answered_even_when_the_queue_is_full() {
    let mut miniq = Miniq::new();
    fifo(&mut miniq, "q.fifo");
    for n in 0..100_000 {
        let response = miniq.call(send_fifo("q.fifo", "x", "g", &format!("d{n}")));
        assert_eq!(response["ok"], true);
    }
    miniq.fails(send_fifo("q.fifo", "x", "g", "new"), "LimitExceeded");
    assert_eq!(
        miniq.ok(send_fifo("q.fifo", "x", "g", "d7")),
        json!({"ok": true, "MessageId": "m-8", "SequenceNumber": 8})
    );
}
