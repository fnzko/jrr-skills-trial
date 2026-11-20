mod common;

use common::Session;
use serde_json::{Value, json};

fn create(s: &mut Session, name: &str, attributes: Value) {
    let line = json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attributes});
    s.ok(&line.to_string());
}

fn send(s: &mut Session, queue: &str, body: &str) {
    s.ok(&json!({"Command": "SendMessage", "QueueName": queue, "Body": body}).to_string());
}

fn tick(s: &mut Session, seconds: u32) {
    s.ok(&format!(r#"{{"Command": "Tick", "Seconds": {seconds}}}"#));
}

/// Receives one message and returns its receipt handle.
fn receive_handle(s: &mut Session, queue: &str) -> String {
    let line = json!({"Command": "ReceiveMessage", "QueueName": queue});
    let response = s.ok(&line.to_string());
    response["Messages"][0]["ReceiptHandle"]
        .as_str()
        .unwrap()
        .to_owned()
}

fn counts(s: &mut Session, queue: &str) -> Value {
    let line = json!({"Command": "GetQueueAttributes", "QueueName": queue});
    s.ok(&line.to_string())["Counts"].clone()
}

fn delete(s: &mut Session, queue: &str, handle: &str) -> Value {
    let line = json!({"Command": "DeleteMessage", "QueueName": queue, "ReceiptHandle": handle});
    s.send(&line.to_string())
}

fn change(s: &mut Session, queue: &str, handle: &str, timeout: u32) -> Value {
    let line = json!({"Command": "ChangeMessageVisibility", "QueueName": queue,
                      "ReceiptHandle": handle, "VisibilityTimeout": timeout});
    s.send(&line.to_string())
}

fn code(response: &Value) -> &str {
    assert_eq!(response["ok"], false, "{response}");
    response["error"]["code"].as_str().unwrap()
}

#[test]
fn delete_removes_the_in_flight_message() {
    let mut s = Session::new();
    create(&mut s, "q", json!({}));
    send(&mut s, "q", "a");
    let handle = receive_handle(&mut s, "q");
    assert_eq!(delete(&mut s, "q", &handle), json!({"ok": true}));
    assert_eq!(
        counts(&mut s, "q"),
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
    tick(&mut s, 100);
    assert_eq!(receive_handle_or_none(&mut s), None);
    // The handle was issued, but its message is gone.
    assert_eq!(code(&delete(&mut s, "q", &handle)), "ReceiptHandleExpired");
}

fn receive_handle_or_none(s: &mut Session) -> Option<String> {
    let response = s.ok(r#"{"Command": "ReceiveMessage", "QueueName": "q"}"#);
    response["Messages"]
        .get(0)
        .map(|m| m["ReceiptHandle"].as_str().unwrap().to_owned())
}

#[test]
fn handles_that_were_never_issued_are_invalid() {
    let mut s = Session::new();
    create(&mut s, "q", json!({}));
    send(&mut s, "q", "a");
    assert_eq!(receive_handle(&mut s, "q"), "r-1");
    for handle in [
        "",
        "garbage",
        "r-0",
        "r-2",
        "r-01",
        "r-+1",
        "R-1",
        "r-1 ",
        " r-1",
        "r--1",
        "r-",
        "m-1",
        "r-99999999999999999999999",
    ] {
        assert_eq!(
            code(&delete(&mut s, "q", handle)),
            "ReceiptHandleInvalid",
            "{handle:?}"
        );
        assert_eq!(
            code(&change(&mut s, "q", handle, 5)),
            "ReceiptHandleInvalid",
            "{handle:?}"
        );
    }
    assert_eq!(delete(&mut s, "q", "r-1"), json!({"ok": true}));
}

#[test]
fn a_redelivered_message_expires_its_earlier_handles() {
    let mut s = Session::new();
    create(&mut s, "q", json!({"VisibilityTimeout": 10}));
    send(&mut s, "q", "a");
    let first = receive_handle(&mut s, "q");
    // Its visibility timeout ended, even before it is delivered again.
    tick(&mut s, 10);
    assert_eq!(code(&delete(&mut s, "q", &first)), "ReceiptHandleExpired");
    let second = receive_handle(&mut s, "q");
    assert_eq!((first.as_str(), second.as_str()), ("r-1", "r-2"));
    assert_eq!(code(&delete(&mut s, "q", &first)), "ReceiptHandleExpired");
    assert_eq!(
        code(&change(&mut s, "q", &first, 5)),
        "ReceiptHandleExpired"
    );
    assert_eq!(delete(&mut s, "q", &second), json!({"ok": true}));
}

#[test]
fn a_zero_visibility_timeout_leaves_no_usable_handle() {
    let mut s = Session::new();
    create(&mut s, "q", json!({"VisibilityTimeout": 0}));
    send(&mut s, "q", "a");
    let handle = receive_handle(&mut s, "q");
    assert_eq!(code(&delete(&mut s, "q", &handle)), "ReceiptHandleExpired");
    assert_eq!(
        counts(&mut s, "q"),
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn a_handle_only_works_in_the_queue_that_issued_it() {
    let mut s = Session::new();
    create(&mut s, "a", json!({}));
    create(&mut s, "b", json!({}));
    send(&mut s, "a", "x");
    let handle = receive_handle(&mut s, "a");
    assert_eq!(code(&delete(&mut s, "b", &handle)), "ReceiptHandleExpired");
    assert_eq!(
        code(&change(&mut s, "b", &handle, 1)),
        "ReceiptHandleExpired"
    );
    assert_eq!(delete(&mut s, "a", &handle), json!({"ok": true}));
}

#[test]
fn a_handle_of_a_deleted_and_recreated_queue_is_expired() {
    let mut s = Session::new();
    create(&mut s, "q", json!({}));
    send(&mut s, "q", "x");
    let handle = receive_handle(&mut s, "q");
    s.ok(r#"{"Command": "DeleteQueue", "QueueName": "q"}"#);
    create(&mut s, "q", json!({}));
    send(&mut s, "q", "x");
    assert_eq!(code(&delete(&mut s, "q", &handle)), "ReceiptHandleExpired");
}

#[test]
fn handles_of_purged_expired_and_dead_lettered_messages_are_expired() {
    let mut s = Session::new();
    create(&mut s, "dead", json!({}));
    create(&mut s, "purged", json!({}));
    create(&mut s, "aging", json!({"RetentionSeconds": 60}));
    create(
        &mut s,
        "moved",
        json!({"DeadLetterQueue": "dead", "MaxReceiveCount": 1, "VisibilityTimeout": 1000}),
    );
    for queue in ["purged", "aging", "moved"] {
        send(&mut s, queue, "x");
    }
    let purged = receive_handle(&mut s, "purged");
    let aging = receive_handle(&mut s, "aging");
    let moved = receive_handle(&mut s, "moved");
    s.ok(r#"{"Command": "PurgeQueue", "QueueName": "purged"}"#);
    tick(&mut s, 60);
    assert_eq!(
        code(&delete(&mut s, "purged", &purged)),
        "ReceiptHandleExpired"
    );
    assert_eq!(
        code(&delete(&mut s, "aging", &aging)),
        "ReceiptHandleExpired"
    );
    // A dead-lettered message was visible when it moved; its old handle is stale.
    s.ok(r#"{"Command": "ChangeMessageVisibility", "QueueName": "moved", "ReceiptHandle": "r-3", "VisibilityTimeout": 0}"#);
    s.ok(r#"{"Command": "ReceiveMessage", "QueueName": "moved"}"#);
    assert_eq!(
        code(&delete(&mut s, "moved", &moved)),
        "ReceiptHandleExpired"
    );
    assert_eq!(
        counts(&mut s, "dead"),
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn change_visibility_replaces_the_end_of_the_timeout() {
    let mut s = Session::new();
    create(&mut s, "q", json!({"VisibilityTimeout": 10}));
    send(&mut s, "q", "a");
    let handle = receive_handle(&mut s, "q");
    tick(&mut s, 5);
    // Extends: in flight until 5 + 20.
    assert_eq!(change(&mut s, "q", &handle, 20), json!({"ok": true}));
    tick(&mut s, 19);
    assert_eq!(
        counts(&mut s, "q"),
        json!({"Visible": 0, "InFlight": 1, "Delayed": 0})
    );
    // Shortens: in flight until 24 + 1.
    assert_eq!(change(&mut s, "q", &handle, 1), json!({"ok": true}));
    tick(&mut s, 1);
    assert_eq!(
        counts(&mut s, "q"),
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(
        code(&change(&mut s, "q", &handle, 1)),
        "ReceiptHandleExpired"
    );
}

#[test]
fn change_visibility_to_zero_makes_the_message_visible_without_a_new_receive_count() {
    let mut s = Session::new();
    create(&mut s, "q", json!({}));
    send(&mut s, "q", "a");
    let handle = receive_handle(&mut s, "q");
    assert_eq!(change(&mut s, "q", &handle, 0), json!({"ok": true}));
    assert_eq!(
        counts(&mut s, "q"),
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
    let again = s.ok(r#"{"Command": "ReceiveMessage", "QueueName": "q"}"#);
    assert_eq!(again["Messages"][0]["ReceiveCount"], 2);
    assert_eq!(again["Messages"][0]["ReceiptHandle"], "r-2");
}

#[test]
fn handle_commands_check_their_parameters_in_order() {
    let mut s = Session::new();
    create(&mut s, "q", json!({}));
    assert_eq!(
        s.err(r#"{"Command": "DeleteMessage", "QueueName": "q"}"#),
        "MissingParameter"
    );
    assert_eq!(
        s.err(r#"{"Command": "DeleteMessage", "ReceiptHandle": "r-1"}"#),
        "MissingParameter"
    );
    assert_eq!(
        s.err(r#"{"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": 1}"#),
        "InvalidParameterValue"
    );
    assert_eq!(
        s.err(
            r#"{"Command": "ChangeMessageVisibility", "QueueName": "q", "ReceiptHandle": "r-1"}"#
        ),
        "MissingParameter"
    );
    for timeout in ["-1", "43201", "1.5", "\"1\""] {
        let line = format!(
            r#"{{"Command": "ChangeMessageVisibility", "QueueName": "nope", "ReceiptHandle": "r-1", "VisibilityTimeout": {timeout}}}"#
        );
        assert_eq!(s.err(&line), "InvalidParameterValue", "{timeout}");
    }
    // The queue lookup comes before the handle checks.
    assert_eq!(
        s.err(r#"{"Command": "DeleteMessage", "QueueName": "nope", "ReceiptHandle": "r-1"}"#),
        "QueueDoesNotExist"
    );
    assert_eq!(
        s.err(r#"{"Command": "ChangeMessageVisibility", "QueueName": "q", "ReceiptHandle": "r-1", "VisibilityTimeout": 43200}"#),
        "ReceiptHandleInvalid"
    );
}
