mod common;

use common::Session;
use serde_json::{Value, json};

fn session_with(queues: &[(&str, Value)]) -> Session {
    let mut s = Session::new();
    for (name, attributes) in queues {
        s.ok(json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attributes}));
    }
    s
}

fn send(s: &mut Session, queue: &str, body: &str) {
    s.ok(json!({"Command": "SendMessage", "QueueName": queue, "Body": body}));
}

fn send_fifo(s: &mut Session, queue: &str, group: &str, body: &str) {
    s.ok(json!({"Command": "SendMessage", "QueueName": queue, "Body": body, "GroupId": group, "DeduplicationId": body}));
}

fn receive(s: &mut Session, queue: &str, extra: Value) -> Vec<Value> {
    let mut command = json!({"Command": "ReceiveMessage", "QueueName": queue});
    command
        .as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    s.ok(command)["Messages"].as_array().unwrap().clone()
}

fn bodies(messages: &[Value]) -> Vec<&str> {
    messages
        .iter()
        .map(|m| m["Body"].as_str().unwrap())
        .collect()
}

fn counts(s: &mut Session, queue: &str) -> Value {
    s.ok(json!({"Command": "GetQueueAttributes", "QueueName": queue}))["Counts"].clone()
}

#[test]
fn receive_delivers_in_arrival_order_with_full_details() {
    let mut s = session_with(&[("q", json!({}))]);
    s.tick(7);
    send(&mut s, "q", "a");
    send(&mut s, "q", "b");
    s.tick(3);
    let got = receive(&mut s, "q", json!({}));
    assert_eq!(
        got,
        vec![
            json!({"MessageId": "m-1", "ReceiptHandle": "r-1", "Body": "a", "ReceiveCount": 1, "SentAt": 7, "FirstReceivedAt": 10})
        ]
    );
    let got = receive(&mut s, "q", json!({"MaxMessages": 10}));
    assert_eq!(bodies(&got), ["b"]);
    assert_eq!(got[0]["ReceiptHandle"], "r-2");
    assert!(receive(&mut s, "q", json!({"MaxMessages": 10})).is_empty());
    assert_eq!(
        counts(&mut s, "q"),
        json!({"Visible": 0, "InFlight": 2, "Delayed": 0})
    );
}

#[test]
fn receive_validates_parameters() {
    let mut s = session_with(&[("q", json!({}))]);
    assert_eq!(
        s.err(json!({"Command": "ReceiveMessage"})),
        "MissingParameter"
    );
    for bad in [json!(0), json!(11), json!("1")] {
        assert_eq!(
            s.err(json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": bad})),
            "InvalidParameterValue"
        );
    }
    assert_eq!(
        s.err(json!({"Command": "ReceiveMessage", "QueueName": "q", "VisibilityTimeout": 43201})),
        "InvalidParameterValue"
    );
    assert_eq!(
        s.err(json!({"Command": "ReceiveMessage", "QueueName": "nope"})),
        "QueueDoesNotExist"
    );
}

#[test]
fn max_messages_limits_a_call() {
    let mut s = session_with(&[("q", json!({}))]);
    for i in 0..5 {
        send(&mut s, "q", &i.to_string());
    }
    assert_eq!(
        bodies(&receive(&mut s, "q", json!({"MaxMessages": 3}))),
        ["0", "1", "2"]
    );
    assert_eq!(
        bodies(&receive(&mut s, "q", json!({"MaxMessages": 3}))),
        ["3", "4"]
    );
}

#[test]
fn visibility_timeout_param_and_default_and_order_after_expiry() {
    let mut s = session_with(&[("q", json!({"VisibilityTimeout": 10}))]);
    send(&mut s, "q", "a");
    send(&mut s, "q", "b");
    assert_eq!(bodies(&receive(&mut s, "q", json!({}))), ["a"]);
    assert_eq!(
        bodies(&receive(&mut s, "q", json!({"VisibilityTimeout": 100}))),
        ["b"]
    );
    s.tick(9);
    assert!(receive(&mut s, "q", json!({})).is_empty());
    s.tick(1);
    let again = receive(&mut s, "q", json!({"MaxMessages": 2}));
    assert_eq!(bodies(&again), ["a"]);
    assert_eq!(again[0]["ReceiveCount"], 2);
    assert_eq!(again[0]["FirstReceivedAt"], 0);
    assert_eq!(again[0]["ReceiptHandle"], "r-3");
}

#[test]
fn zero_visibility_makes_the_message_visible_at_once_with_a_dead_handle() {
    let mut s = session_with(&[("q", json!({}))]);
    send(&mut s, "q", "a");
    let got = receive(&mut s, "q", json!({"VisibilityTimeout": 0}));
    let handle = got[0]["ReceiptHandle"].clone();
    assert_eq!(
        s.err(json!({"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": handle})),
        "ReceiptHandleExpired"
    );
    assert_eq!(receive(&mut s, "q", json!({}))[0]["ReceiveCount"], 2);
}

#[test]
fn delayed_messages_are_skipped_but_keep_their_place() {
    let mut s = session_with(&[("q", json!({}))]);
    s.ok(json!({"Command": "SendMessage", "QueueName": "q", "Body": "late", "DelaySeconds": 5}));
    send(&mut s, "q", "now");
    assert_eq!(
        bodies(&receive(&mut s, "q", json!({"MaxMessages": 5}))),
        ["now"]
    );
    s.tick(5);
    assert_eq!(
        bodies(&receive(&mut s, "q", json!({"MaxMessages": 5}))),
        ["late"]
    );
}

#[test]
fn fifo_delivers_one_message_per_group_and_blocks_busy_groups() {
    let mut s = session_with(&[("f.fifo", json!({"Fifo": true}))]);
    for (group, body) in [
        ("a", "a1"),
        ("b", "b1"),
        ("a", "a2"),
        ("c", "c1"),
        ("b", "b2"),
    ] {
        send_fifo(&mut s, "f.fifo", group, body);
    }
    let got = receive(&mut s, "f.fifo", json!({"MaxMessages": 10}));
    assert_eq!(bodies(&got), ["a1", "b1", "c1"]);
    assert_eq!(got[1]["GroupId"], "b");
    assert_eq!(got[1]["SequenceNumber"], 2);
    // Everything left sits behind an in-flight message of its group.
    assert!(receive(&mut s, "f.fifo", json!({"MaxMessages": 10})).is_empty());
    // Delete a1: group a unblocks, b and c stay blocked.
    s.ok(json!({"Command": "DeleteMessage", "QueueName": "f.fifo", "ReceiptHandle": got[0]["ReceiptHandle"]}));
    assert_eq!(
        bodies(&receive(&mut s, "f.fifo", json!({"MaxMessages": 10}))),
        ["a2"]
    );
}

#[test]
fn fifo_message_keeps_its_place_after_visibility_timeout_ends() {
    let mut s = session_with(&[("f.fifo", json!({"Fifo": true, "VisibilityTimeout": 5}))]);
    send_fifo(&mut s, "f.fifo", "g", "one");
    send_fifo(&mut s, "f.fifo", "g", "two");
    assert_eq!(bodies(&receive(&mut s, "f.fifo", json!({}))), ["one"]);
    s.tick(5);
    // "one" is visible again and keeps its place ahead of "two".
    assert_eq!(
        bodies(&receive(&mut s, "f.fifo", json!({"MaxMessages": 2}))),
        ["one"]
    );
}

#[test]
fn exhausted_messages_move_to_the_dead_letter_queue() {
    let mut s = session_with(&[
        ("dlq", json!({})),
        (
            "q",
            json!({"DeadLetterQueue": "dlq", "MaxReceiveCount": 2, "VisibilityTimeout": 0}),
        ),
    ]);
    send(&mut s, "q", "poison");
    send(&mut s, "q", "fine");
    assert_eq!(bodies(&receive(&mut s, "q", json!({}))), ["poison"]);
    assert_eq!(bodies(&receive(&mut s, "q", json!({}))), ["poison"]);
    // The third call dead-letters "poison" without counting it, and delivers "fine".
    assert_eq!(bodies(&receive(&mut s, "q", json!({}))), ["fine"]);
    assert_eq!(
        counts(&mut s, "dlq"),
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
    let moved = receive(&mut s, "dlq", json!({}));
    assert_eq!(moved.len(), 1);
    assert_eq!(moved[0]["MessageId"], "m-1");
    assert_eq!(moved[0]["SentAt"], 0);
    assert_eq!(moved[0]["ReceiveCount"], 1);
    assert_eq!(moved[0]["FirstReceivedAt"], 0);
}

#[test]
fn dead_lettering_does_not_count_toward_max_messages() {
    let mut s = session_with(&[
        ("dlq", json!({})),
        (
            "q",
            json!({"DeadLetterQueue": "dlq", "MaxReceiveCount": 1, "VisibilityTimeout": 0}),
        ),
    ]);
    send(&mut s, "q", "a");
    send(&mut s, "q", "b");
    send(&mut s, "q", "c");
    assert_eq!(
        bodies(&receive(&mut s, "q", json!({"MaxMessages": 2}))),
        ["a", "b"]
    );
    // a and b are dead-lettered; c is delivered.
    assert_eq!(
        bodies(&receive(&mut s, "q", json!({"MaxMessages": 1}))),
        ["c"]
    );
    assert_eq!(
        bodies(&receive(&mut s, "dlq", json!({"MaxMessages": 10}))),
        ["a", "b"]
    );
}

#[test]
fn dead_letter_move_into_fifo_queue_assigns_new_sequence_and_group_order() {
    let mut s = session_with(&[
        ("dlq.fifo", json!({"Fifo": true})),
        (
            "q.fifo",
            json!({"Fifo": true, "DeadLetterQueue": "dlq.fifo", "MaxReceiveCount": 1, "VisibilityTimeout": 0}),
        ),
    ]);
    send_fifo(&mut s, "dlq.fifo", "g", "existing");
    send_fifo(&mut s, "q.fifo", "g", "first");
    send_fifo(&mut s, "q.fifo", "g", "second");
    assert_eq!(bodies(&receive(&mut s, "q.fifo", json!({}))), ["first"]);
    // "first" is dead-lettered, which unblocks "second" within the same call.
    assert_eq!(bodies(&receive(&mut s, "q.fifo", json!({}))), ["second"]);
    let got = receive(&mut s, "dlq.fifo", json!({"MaxMessages": 10}));
    assert_eq!(bodies(&got), ["existing"]);
    s.tick(0);
    let r = s.ok(json!({"Command": "GetQueueAttributes", "QueueName": "dlq.fifo"}));
    assert_eq!(
        r["Counts"],
        json!({"Visible": 1, "InFlight": 1, "Delayed": 0})
    );
    s.ok(json!({"Command": "SendMessage", "QueueName": "dlq.fifo", "Body": "next", "GroupId": "h", "DeduplicationId": "n"}));
}

#[test]
fn dead_letter_move_ignores_dedup_and_applies_target_retention() {
    let mut s = session_with(&[
        ("dlq", json!({"RetentionSeconds": 60})),
        (
            "q",
            json!({"DeadLetterQueue": "dlq", "MaxReceiveCount": 1, "VisibilityTimeout": 0}),
        ),
    ]);
    send(&mut s, "q", "old");
    s.tick(60);
    assert_eq!(bodies(&receive(&mut s, "q", json!({}))), ["old"]);
    assert!(receive(&mut s, "q", json!({})).is_empty());
    // Too old for the dead-letter queue: removed on arrival.
    assert_eq!(
        counts(&mut s, "dlq"),
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn new_dead_letter_configuration_applies_from_the_next_receive() {
    let mut s = session_with(&[("dlq", json!({})), ("q", json!({"VisibilityTimeout": 0}))]);
    send(&mut s, "q", "a");
    receive(&mut s, "q", json!({}));
    s.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"DeadLetterQueue": "dlq", "MaxReceiveCount": 1}}));
    assert!(receive(&mut s, "q", json!({})).is_empty());
    assert_eq!(counts(&mut s, "dlq")["Visible"], 1);
}

#[test]
fn delete_message_rules() {
    let mut s = session_with(&[("q", json!({})), ("other", json!({}))]);
    send(&mut s, "q", "a");
    send(&mut s, "q", "b");
    let got = receive(&mut s, "q", json!({"MaxMessages": 2}));
    let delete = |handle: &str, queue: &str| json!({"Command": "DeleteMessage", "QueueName": queue, "ReceiptHandle": handle});
    assert_eq!(
        s.err(json!({"Command": "DeleteMessage", "QueueName": "q"})),
        "MissingParameter"
    );
    assert_eq!(
        s.err(json!({"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": 1})),
        "InvalidParameterValue"
    );
    assert_eq!(s.err(delete("r-1", "nope")), "QueueDoesNotExist");
    for never in [
        "r-01",
        "R-1",
        "r-1 ",
        "r-0",
        "r-3",
        "r-",
        "1",
        "",
        "r--1",
        "r-+1",
        "r-99999999999999999999999",
    ] {
        assert_eq!(
            s.err(delete(never, "q")),
            "ReceiptHandleInvalid",
            "{never:?}"
        );
    }
    // A handle issued for a message of another queue is expired, not invalid.
    assert_eq!(s.err(delete("r-1", "other")), "ReceiptHandleExpired");
    s.ok(delete("r-1", "q"));
    assert_eq!(s.err(delete("r-1", "q")), "ReceiptHandleExpired");
    // Redelivery makes the old handle stale and the new one latest.
    s.tick(30);
    let again = receive(&mut s, "q", json!({}));
    assert_eq!(again[0]["ReceiptHandle"], "r-3");
    assert_eq!(
        s.err(delete(got[1]["ReceiptHandle"].as_str().unwrap(), "q")),
        "ReceiptHandleExpired"
    );
    s.ok(delete("r-3", "q"));
    assert_eq!(
        counts(&mut s, "q"),
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn handles_die_with_purge_expiry_and_queue_replacement() {
    let mut s = session_with(&[(
        "q",
        json!({"RetentionSeconds": 100, "VisibilityTimeout": 1000}),
    )]);
    let delete =
        |q: &str, h: &str| json!({"Command": "DeleteMessage", "QueueName": q, "ReceiptHandle": h});
    send(&mut s, "q", "a");
    receive(&mut s, "q", json!({}));
    s.ok(json!({"Command": "PurgeQueue", "QueueName": "q"}));
    assert_eq!(s.err(delete("q", "r-1")), "ReceiptHandleExpired");

    send(&mut s, "q", "b");
    receive(&mut s, "q", json!({}));
    s.tick(100);
    assert_eq!(s.err(delete("q", "r-2")), "ReceiptHandleExpired");

    send(&mut s, "q", "c");
    receive(&mut s, "q", json!({}));
    s.ok(json!({"Command": "DeleteQueue", "QueueName": "q"}));
    s.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    assert_eq!(s.err(delete("q", "r-3")), "ReceiptHandleExpired");
}

#[test]
fn change_message_visibility_rules_and_effects() {
    let mut s = session_with(&[("q", json!({}))]);
    send(&mut s, "q", "a");
    let change = |handle: &str, timeout: Value| json!({"Command": "ChangeMessageVisibility", "QueueName": "q", "ReceiptHandle": handle, "VisibilityTimeout": timeout});
    assert_eq!(
        s.err(
            json!({"Command": "ChangeMessageVisibility", "QueueName": "q", "ReceiptHandle": "r-1"})
        ),
        "MissingParameter"
    );
    assert_eq!(s.err(change("r-1", json!(43201))), "InvalidParameterValue");
    assert_eq!(s.err(change("r-1", json!(5))), "ReceiptHandleInvalid");
    receive(&mut s, "q", json!({}));
    // Extend: 20 seconds from now, not from the delivery.
    s.tick(10);
    s.ok(change("r-1", json!(20)));
    s.tick(19);
    assert!(receive(&mut s, "q", json!({})).is_empty());
    s.tick(1);
    // The timeout ended, so the handle is expired and cannot be changed.
    assert_eq!(s.err(change("r-1", json!(5))), "ReceiptHandleExpired");
    let again = receive(&mut s, "q", json!({}));
    assert_eq!(again[0]["ReceiveCount"], 2);
    // Zero makes it visible at once without touching ReceiveCount.
    s.ok(change("r-2", json!(0)));
    assert_eq!(
        counts(&mut s, "q"),
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(receive(&mut s, "q", json!({}))[0]["ReceiveCount"], 3);
}
