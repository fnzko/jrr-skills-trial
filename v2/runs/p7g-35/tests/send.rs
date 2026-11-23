mod common;

use common::Session;
use serde_json::{json, Value};

fn create(s: &mut Session, name: &str, attributes: Value) {
    s.ok(json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attributes}));
}

fn counts(s: &mut Session, name: &str) -> Value {
    s.ok(json!({"Command": "GetQueueAttributes", "QueueName": name}))["Counts"].clone()
}

fn send(queue: &str, body: &str) -> Value {
    json!({"Command": "SendMessage", "QueueName": queue, "Body": body})
}

fn fifo_send(queue: &str, body: &str, group: &str, dedup: &str) -> Value {
    json!({"Command": "SendMessage", "QueueName": queue, "Body": body,
           "GroupId": group, "DeduplicationId": dedup})
}

#[test]
fn message_ids_count_accepted_messages_across_queues() {
    let mut s = Session::new();
    create(&mut s, "a", json!({}));
    create(&mut s, "b", json!({}));
    assert_eq!(
        s.ok(send("a", "x")),
        json!({"ok": true, "MessageId": "m-1"})
    );
    assert_eq!(s.ok(send("b", "x"))["MessageId"], "m-2");
    s.err(send("missing", "x"), "QueueDoesNotExist");
    s.err(send("a", ""), "InvalidParameterValue");
    assert_eq!(s.ok(send("a", "x"))["MessageId"], "m-3");
}

#[test]
fn body_length_is_counted_in_utf8_bytes() {
    let mut s = Session::new();
    create(&mut s, "q", json!({}));
    s.ok(send("q", &"a".repeat(1024)));
    s.ok(send("q", &"\u{e9}".repeat(512)));
    s.err(send("q", &"a".repeat(1025)), "InvalidParameterValue");
    s.err(send("q", &"\u{e9}".repeat(513)), "InvalidParameterValue");
    s.err(
        json!({"Command": "SendMessage", "QueueName": "q", "Body": 5}),
        "InvalidParameterValue",
    );
    s.err(
        json!({"Command": "SendMessage", "QueueName": "q"}),
        "MissingParameter",
    );
    s.err(
        json!({"Command": "SendMessage", "Body": "x"}),
        "MissingParameter",
    );
}

#[test]
fn validation_precedes_the_queue_lookup() {
    let mut s = Session::new();
    s.err(
        json!({"Command": "SendMessage", "QueueName": "nope", "Body": "x", "DelaySeconds": 901}),
        "InvalidParameterValue",
    );
    s.err(
        json!({"Command": "SendMessage", "QueueName": "nope"}),
        "MissingParameter",
    );
    s.err(
        json!({"Command": "SendMessage", "QueueName": "nope", "Body": "", "DelaySeconds": "1"}),
        "InvalidParameterValue",
    );
    s.err(
        json!({"Command": "SendMessage", "QueueName": 1, "Body": 1}),
        "InvalidParameterValue",
    );
}

#[test]
fn delay_comes_from_the_request_or_else_from_the_queue() {
    let mut s = Session::new();
    create(&mut s, "q", json!({"DelaySeconds": 10}));
    s.ok(send("q", "queue default"));
    s.ok(json!({"Command": "SendMessage", "QueueName": "q", "Body": "none", "DelaySeconds": 0}));
    s.ok(json!({"Command": "SendMessage", "QueueName": "q", "Body": "long", "DelaySeconds": 900}));
    assert_eq!(
        counts(&mut s, "q"),
        json!({"Visible": 1, "InFlight": 0, "Delayed": 2})
    );
    s.tick(9);
    assert_eq!(
        counts(&mut s, "q"),
        json!({"Visible": 1, "InFlight": 0, "Delayed": 2})
    );
    s.tick(1);
    assert_eq!(
        counts(&mut s, "q"),
        json!({"Visible": 2, "InFlight": 0, "Delayed": 1})
    );
    s.tick(890);
    assert_eq!(
        counts(&mut s, "q"),
        json!({"Visible": 3, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn a_changed_queue_delay_only_affects_later_sends() {
    let mut s = Session::new();
    create(&mut s, "q", json!({"DelaySeconds": 10}));
    s.ok(send("q", "a"));
    s.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"DelaySeconds": 0}}));
    s.ok(send("q", "b"));
    assert_eq!(
        counts(&mut s, "q"),
        json!({"Visible": 1, "InFlight": 0, "Delayed": 1})
    );
}

#[test]
fn standard_and_fifo_queues_accept_different_parameters() {
    let mut s = Session::new();
    create(&mut s, "std", json!({}));
    create(&mut s, "f.fifo", json!({"Fifo": true}));
    for extra in [json!({"GroupId": "g"}), json!({"DeduplicationId": "d"})] {
        let mut request = send("std", "x");
        request
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        s.err(request, "UnsupportedOperation");
    }
    let mut request = fifo_send("f.fifo", "x", "g", "d");
    request["DelaySeconds"] = json!(0);
    s.err(request, "UnsupportedOperation");
    s.err(json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "x", "DeduplicationId": "d"}), "MissingParameter");
    s.err(
        json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "x", "GroupId": "g"}),
        "MissingParameter",
    );
    assert_eq!(
        counts(&mut s, "std"),
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
    // Unsupported wins over missing, and null counts as absent.
    s.err(
        json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "x", "DelaySeconds": 5}),
        "UnsupportedOperation",
    );
    s.ok(json!({"Command": "SendMessage", "QueueName": "std", "Body": "x", "GroupId": null, "DeduplicationId": null}));
}

#[test]
fn group_and_deduplication_ids_are_validated() {
    let mut s = Session::new();
    create(&mut s, "f.fifo", json!({"Fifo": true}));
    let ok = "a".repeat(128);
    s.ok(fifo_send("f.fifo", "x", &ok, &ok));
    for bad in ["", "a b", "a.b", "\u{e9}", &"a".repeat(129)] {
        s.err(fifo_send("f.fifo", "x", bad, "d"), "InvalidParameterValue");
        s.err(fifo_send("f.fifo", "x", "g", bad), "InvalidParameterValue");
    }
}

#[test]
fn fifo_sends_get_per_queue_sequence_numbers() {
    let mut s = Session::new();
    create(&mut s, "a.fifo", json!({"Fifo": true}));
    create(&mut s, "b.fifo", json!({"Fifo": true}));
    let first = s.ok(fifo_send("a.fifo", "x", "g", "1"));
    assert_eq!(
        first,
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
    assert_eq!(
        s.ok(fifo_send("b.fifo", "x", "g", "1")),
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 1})
    );
    assert_eq!(
        s.ok(fifo_send("a.fifo", "x", "h", "2"))["SequenceNumber"],
        2
    );
}

#[test]
fn duplicate_sends_return_the_original_and_use_no_number() {
    let mut s = Session::new();
    create(&mut s, "f.fifo", json!({"Fifo": true}));
    let first = s.ok(fifo_send("f.fifo", "one", "g", "d"));
    let again = s.ok(fifo_send("f.fifo", "two", "other-group", "d"));
    assert_eq!(again, first);
    assert_eq!(
        s.ok(fifo_send("f.fifo", "three", "g", "e"))["MessageId"],
        "m-2"
    );
    assert_eq!(counts(&mut s, "f.fifo")["Visible"], 2);
}

#[test]
fn the_deduplication_window_is_300_seconds_and_is_not_extended() {
    let mut s = Session::new();
    create(&mut s, "f.fifo", json!({"Fifo": true}));
    s.ok(fifo_send("f.fifo", "x", "g", "d"));
    s.tick(299);
    assert_eq!(s.ok(fifo_send("f.fifo", "x", "g", "d"))["MessageId"], "m-1");
    s.tick(1);
    // t = 300: the first window is over; this send opens a new one.
    let accepted = s.ok(fifo_send("f.fifo", "x", "g", "d"));
    assert_eq!(
        (
            accepted["MessageId"].clone(),
            accepted["SequenceNumber"].clone()
        ),
        (json!("m-2"), json!(2))
    );
    s.tick(299);
    assert_eq!(s.ok(fifo_send("f.fifo", "x", "g", "d"))["MessageId"], "m-2");
}

#[test]
fn deduplication_ids_outlive_messages_and_survive_purge() {
    let mut s = Session::new();
    create(
        &mut s,
        "f.fifo",
        json!({"Fifo": true, "RetentionSeconds": 60}),
    );
    s.ok(fifo_send("f.fifo", "x", "g", "d"));
    s.ok(json!({"Command": "PurgeQueue", "QueueName": "f.fifo"}));
    assert_eq!(s.ok(fifo_send("f.fifo", "x", "g", "d"))["MessageId"], "m-1");
    assert_eq!(counts(&mut s, "f.fifo")["Visible"], 0);
    s.tick(100);
    assert_eq!(s.ok(fifo_send("f.fifo", "x", "g", "d"))["MessageId"], "m-1");
    // Sequence numbers carry on after a purge.
    assert_eq!(
        s.ok(fifo_send("f.fifo", "y", "g", "e"))["SequenceNumber"],
        2
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
    let body_only = |body: &str| json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": body, "GroupId": "g"});
    assert_eq!(s.ok(body_only("same"))["MessageId"], "m-1");
    assert_eq!(s.ok(body_only("same"))["MessageId"], "m-1");
    assert_eq!(s.ok(body_only("other"))["MessageId"], "m-2");
    // An explicit id is a different key from the body used as an id...
    assert_eq!(
        s.ok(fifo_send("f.fifo", "same", "g", "explicit"))["MessageId"],
        "m-3"
    );
    // ...but an id equal to a body collides with it.
    assert_eq!(
        s.ok(fifo_send("f.fifo", "unrelated", "g", "same"))["MessageId"],
        "m-1"
    );
    let long = "z".repeat(1024);
    assert_eq!(s.ok(body_only(&long))["MessageId"], "m-4");
    assert_eq!(s.ok(body_only(&long))["MessageId"], "m-4");
}

#[test]
fn a_queue_holds_at_most_100000_messages() {
    let mut s = Session::new();
    create(&mut s, "q", json!({}));
    for i in 0..100_000 {
        let r = s.send(send("q", "x"));
        assert_eq!(r["ok"], true, "send {i}");
    }
    s.err(send("q", "x"), "LimitExceeded");
    s.ok(json!({"Command": "PurgeQueue", "QueueName": "q"}));
    assert_eq!(s.ok(send("q", "x"))["MessageId"], "m-100001");
}

#[test]
fn limit_is_checked_after_duplicates() {
    let mut s = Session::new();
    create(&mut s, "f.fifo", json!({"Fifo": true}));
    for i in 0..100_000 {
        let r = s.send(fifo_send("f.fifo", "x", "g", &format!("d{i}")));
        assert_eq!(r["ok"], true, "send {i}");
    }
    assert_eq!(
        s.ok(fifo_send("f.fifo", "x", "g", "d5"))["MessageId"],
        "m-6"
    );
    s.err(fifo_send("f.fifo", "x", "g", "new"), "LimitExceeded");
}

#[test]
fn retention_removes_messages_whatever_their_state() {
    let mut s = Session::new();
    create(&mut s, "q", json!({"RetentionSeconds": 100}));
    s.ok(send("q", "a"));
    s.tick(50);
    s.ok(json!({"Command": "SendMessage", "QueueName": "q", "Body": "b", "DelaySeconds": 900}));
    s.tick(49);
    assert_eq!(
        counts(&mut s, "q"),
        json!({"Visible": 1, "InFlight": 0, "Delayed": 1})
    );
    s.tick(1);
    assert_eq!(
        counts(&mut s, "q"),
        json!({"Visible": 0, "InFlight": 0, "Delayed": 1})
    );
    s.tick(50);
    assert_eq!(
        counts(&mut s, "q"),
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn lowering_retention_removes_at_once_and_raising_it_restores_nothing() {
    let mut s = Session::new();
    create(&mut s, "q", json!({}));
    s.ok(send("q", "old"));
    s.tick(100);
    s.ok(send("q", "new"));
    let set = |seconds: u64| json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"RetentionSeconds": seconds}});
    s.ok(set(100));
    assert_eq!(counts(&mut s, "q")["Visible"], 1);
    s.ok(set(1_209_600));
    assert_eq!(counts(&mut s, "q")["Visible"], 1);
}

#[test]
fn messages_that_expired_before_a_retention_change_stay_gone() {
    let mut s = Session::new();
    create(&mut s, "q", json!({"RetentionSeconds": 60}));
    s.ok(send("q", "old"));
    s.tick(60);
    // Nothing has touched the queue since the message expired.
    s.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"RetentionSeconds": 1000}}));
    assert_eq!(counts(&mut s, "q")["Visible"], 0);
}
