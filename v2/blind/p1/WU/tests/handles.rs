mod common;

use common::Miniq;
use serde_json::{Value, json};

fn create(miniq: &mut Miniq, name: &str, attributes: Value) {
    miniq.ok(json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attributes}));
}

fn send(miniq: &mut Miniq, queue: &str, body: &str) {
    miniq.ok(json!({"Command": "SendMessage", "QueueName": queue, "Body": body}));
}

fn tick(miniq: &mut Miniq, seconds: u64) {
    miniq.ok(json!({"Command": "Tick", "Seconds": seconds}));
}

/// Receives one message and returns its handle.
fn receive_handle(miniq: &mut Miniq, queue: &str) -> String {
    let response = miniq.ok(json!({"Command": "ReceiveMessage", "QueueName": queue}));
    response["Messages"][0]["ReceiptHandle"]
        .as_str()
        .unwrap()
        .to_owned()
}

fn delete(queue: &str, handle: &str) -> Value {
    json!({"Command": "DeleteMessage", "QueueName": queue, "ReceiptHandle": handle})
}

fn change(queue: &str, handle: &str, timeout: u64) -> Value {
    json!({"Command": "ChangeMessageVisibility", "QueueName": queue, "ReceiptHandle": handle, "VisibilityTimeout": timeout})
}

fn counts(miniq: &mut Miniq, queue: &str) -> Value {
    miniq.ok(json!({"Command": "GetQueueAttributes", "QueueName": queue}))["Counts"].clone()
}

#[test]
fn delete_removes_the_message_and_the_handle_is_then_expired() {
    let mut miniq = Miniq::new();
    create(&mut miniq, "q", json!({}));
    send(&mut miniq, "q", "a");
    let handle = receive_handle(&mut miniq, "q");
    assert_eq!(miniq.ok(delete("q", &handle)), json!({"ok": true}));
    assert_eq!(
        counts(&mut miniq, "q"),
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
    miniq.fails(delete("q", &handle), "ReceiptHandleExpired");
    miniq.fails(change("q", &handle, 5), "ReceiptHandleExpired");
}

#[test]
fn handles_never_issued_are_invalid() {
    let mut miniq = Miniq::new();
    create(&mut miniq, "q", json!({}));
    send(&mut miniq, "q", "a");
    assert_eq!(receive_handle(&mut miniq, "q"), "r-1");
    for handle in [
        "r-01",
        "R-1",
        "r-1 ",
        " r-1",
        "r-0",
        "r-2",
        "r-",
        "r",
        "",
        "1",
        "r--1",
        "r-+1",
        "r-1.0",
        "r-99999999999999999999999",
        "\u{ff11}",
    ] {
        miniq.fails(delete("q", handle), "ReceiptHandleInvalid");
        miniq.fails(change("q", handle, 1), "ReceiptHandleInvalid");
    }
    miniq.ok(delete("q", "r-1"));
}

#[test]
fn parameter_errors_come_before_handle_checks() {
    let mut miniq = Miniq::new();
    create(&mut miniq, "q", json!({}));
    miniq.fails(
        json!({"Command": "DeleteMessage", "QueueName": "q"}),
        "MissingParameter",
    );
    miniq.fails(
        json!({"Command": "DeleteMessage", "ReceiptHandle": "r-1"}),
        "MissingParameter",
    );
    miniq.fails(
        json!({"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": 1}),
        "InvalidParameterValue",
    );
    miniq.fails(
        json!({"Command": "ChangeMessageVisibility", "QueueName": "q", "ReceiptHandle": "r-1"}),
        "MissingParameter",
    );
    for timeout in [json!(-1), json!(43201), json!("5"), json!(1.0)] {
        miniq.fails(
            json!({"Command": "ChangeMessageVisibility", "QueueName": "q", "ReceiptHandle": "r-1", "VisibilityTimeout": timeout}),
            "InvalidParameterValue",
        );
    }
    // The queue is looked up before the handle is judged.
    miniq.fails(delete("nope", "garbage"), "QueueDoesNotExist");
    miniq.fails(change("nope", "garbage", 1), "QueueDoesNotExist");
    miniq.fails(delete("q", "garbage"), "ReceiptHandleInvalid");
}

#[test]
fn a_handle_is_expired_once_the_message_is_delivered_again() {
    let mut miniq = Miniq::new();
    create(&mut miniq, "q", json!({"VisibilityTimeout": 5}));
    send(&mut miniq, "q", "a");
    let old = receive_handle(&mut miniq, "q");
    tick(&mut miniq, 5);
    miniq.fails(delete("q", &old), "ReceiptHandleExpired");
    let new = receive_handle(&mut miniq, "q");
    assert_ne!(old, new);
    miniq.fails(delete("q", &old), "ReceiptHandleExpired");
    miniq.fails(change("q", &old, 1), "ReceiptHandleExpired");
    miniq.ok(delete("q", &new));
}

#[test]
fn a_handle_is_usable_until_the_last_second_of_the_timeout() {
    let mut miniq = Miniq::new();
    create(&mut miniq, "q", json!({"VisibilityTimeout": 5}));
    send(&mut miniq, "q", "a");
    let handle = receive_handle(&mut miniq, "q");
    tick(&mut miniq, 4);
    miniq.ok(change("q", &handle, 5));
    tick(&mut miniq, 4);
    miniq.ok(change("q", &handle, 1));
    tick(&mut miniq, 1);
    miniq.fails(delete("q", &handle), "ReceiptHandleExpired");
}

#[test]
fn a_zero_visibility_timeout_leaves_no_usable_handle() {
    let mut miniq = Miniq::new();
    create(&mut miniq, "q", json!({}));
    send(&mut miniq, "q", "a");
    let response =
        miniq.ok(json!({"Command": "ReceiveMessage", "QueueName": "q", "VisibilityTimeout": 0}));
    let handle = response["Messages"][0]["ReceiptHandle"]
        .as_str()
        .unwrap()
        .to_owned();
    miniq.fails(delete("q", &handle), "ReceiptHandleExpired");
    assert_eq!(counts(&mut miniq, "q")["Visible"], 1);
}

#[test]
fn handles_of_other_queues_and_of_gone_messages_are_expired() {
    let mut miniq = Miniq::new();
    create(&mut miniq, "a", json!({}));
    create(&mut miniq, "b", json!({}));
    send(&mut miniq, "a", "x");
    send(&mut miniq, "b", "y");
    let in_a = receive_handle(&mut miniq, "a");
    let in_b = receive_handle(&mut miniq, "b");
    miniq.fails(delete("b", &in_a), "ReceiptHandleExpired");
    miniq.fails(change("a", &in_b, 5), "ReceiptHandleExpired");
    assert_eq!(counts(&mut miniq, "a")["InFlight"], 1);

    miniq.ok(json!({"Command": "PurgeQueue", "QueueName": "a"}));
    miniq.fails(delete("a", &in_a), "ReceiptHandleExpired");

    miniq.ok(json!({"Command": "DeleteQueue", "QueueName": "b"}));
    create(&mut miniq, "b", json!({}));
    send(&mut miniq, "b", "y");
    miniq.fails(delete("b", &in_b), "ReceiptHandleExpired");
}

#[test]
fn a_handle_of_a_message_that_expired_by_retention_is_expired() {
    let mut miniq = Miniq::new();
    create(
        &mut miniq,
        "q",
        json!({"RetentionSeconds": 60, "VisibilityTimeout": 1000}),
    );
    send(&mut miniq, "q", "a");
    let handle = receive_handle(&mut miniq, "q");
    tick(&mut miniq, 60);
    miniq.fails(delete("q", &handle), "ReceiptHandleExpired");
}

#[test]
fn a_handle_of_a_message_moved_to_a_dead_letter_queue_is_expired() {
    let mut miniq = Miniq::new();
    create(&mut miniq, "dead", json!({}));
    create(
        &mut miniq,
        "q",
        json!({"VisibilityTimeout": 5, "DeadLetterQueue": "dead", "MaxReceiveCount": 1}),
    );
    send(&mut miniq, "q", "a");
    let handle = receive_handle(&mut miniq, "q");
    tick(&mut miniq, 5);
    miniq.ok(json!({"Command": "ReceiveMessage", "QueueName": "q"}));
    miniq.fails(delete("q", &handle), "ReceiptHandleExpired");
    miniq.fails(delete("dead", &handle), "ReceiptHandleExpired");
}

#[test]
fn change_visibility_counts_from_now_and_keeps_the_receive_count() {
    let mut miniq = Miniq::new();
    create(&mut miniq, "q", json!({"VisibilityTimeout": 10}));
    send(&mut miniq, "q", "a");
    let handle = receive_handle(&mut miniq, "q");
    tick(&mut miniq, 6);
    assert_eq!(miniq.ok(change("q", &handle, 10)), json!({"ok": true}));
    tick(&mut miniq, 9);
    assert_eq!(counts(&mut miniq, "q")["InFlight"], 1);
    tick(&mut miniq, 1);
    assert_eq!(
        counts(&mut miniq, "q"),
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
    let again = miniq.ok(json!({"Command": "ReceiveMessage", "QueueName": "q"}));
    assert_eq!(again["Messages"][0]["ReceiveCount"], 2);
}

#[test]
fn change_visibility_to_zero_makes_the_message_visible_at_once() {
    let mut miniq = Miniq::new();
    create(&mut miniq, "q", json!({}));
    send(&mut miniq, "q", "a");
    let handle = receive_handle(&mut miniq, "q");
    miniq.ok(change("q", &handle, 0));
    assert_eq!(
        counts(&mut miniq, "q"),
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
    miniq.fails(delete("q", &handle), "ReceiptHandleExpired");
}

#[test]
fn change_visibility_can_shorten_and_lengthen() {
    let mut miniq = Miniq::new();
    create(&mut miniq, "q", json!({"VisibilityTimeout": 100}));
    send(&mut miniq, "q", "a");
    let handle = receive_handle(&mut miniq, "q");
    miniq.ok(change("q", &handle, 2));
    tick(&mut miniq, 1);
    miniq.ok(change("q", &handle, 43200));
    tick(&mut miniq, 43199);
    assert_eq!(counts(&mut miniq, "q")["InFlight"], 1);
    tick(&mut miniq, 1);
    assert_eq!(counts(&mut miniq, "q")["Visible"], 1);
}

#[test]
fn deleting_the_head_of_a_fifo_group_frees_the_next_message() {
    let mut miniq = Miniq::new();
    create(&mut miniq, "q.fifo", json!({"Fifo": true}));
    for body in ["a1", "a2"] {
        miniq.ok(json!({"Command": "SendMessage", "QueueName": "q.fifo", "Body": body, "GroupId": "A", "DeduplicationId": body}));
    }
    let handle = receive_handle(&mut miniq, "q.fifo");
    miniq.ok(delete("q.fifo", &handle));
    let next = miniq.ok(json!({"Command": "ReceiveMessage", "QueueName": "q.fifo"}));
    assert_eq!(next["Messages"][0]["Body"], "a2");
}
