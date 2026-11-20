//! DeleteMessage and ChangeMessageVisibility (section 6), and which receipt
//! handles they accept.

mod common;

use common::{fail, ok};
use miniq::Broker;
use serde_json::{json, Value};

fn receive_one(broker: &mut Broker, queue: &str) -> String {
    let response = ok(
        broker,
        &format!(r#"{{"Command": "ReceiveMessage", "QueueName": "{queue}"}}"#),
    );
    response["Messages"][0]["ReceiptHandle"]
        .as_str()
        .expect("one message delivered")
        .to_owned()
}

fn delete(broker: &mut Broker, queue: &str, handle: &str) -> Value {
    let line = json!({"Command": "DeleteMessage", "QueueName": queue, "ReceiptHandle": handle})
        .to_string();
    common::send(broker, &line)
}

fn counts(broker: &mut Broker, queue: &str) -> Value {
    ok(
        broker,
        &format!(r#"{{"Command": "GetQueueAttributes", "QueueName": "{queue}"}}"#),
    )["Counts"]
        .clone()
}

fn queue_with_one_message(broker: &mut Broker, queue: &str) {
    ok(
        broker,
        &format!(r#"{{"Command": "CreateQueue", "QueueName": "{queue}"}}"#),
    );
    ok(
        broker,
        &format!(r#"{{"Command": "SendMessage", "QueueName": "{queue}", "Body": "x"}}"#),
    );
}

#[test]
fn a_handle_that_was_never_issued_is_invalid() {
    let mut broker = Broker::new();
    queue_with_one_message(&mut broker, "a");
    fail(
        &mut broker,
        r#"{"Command": "DeleteMessage", "QueueName": "a", "ReceiptHandle": "r-1"}"#,
        "ReceiptHandleInvalid",
    );
    fail(
        &mut broker,
        r#"{"Command": "DeleteMessage", "QueueName": "a", "ReceiptHandle": "junk"}"#,
        "ReceiptHandleInvalid",
    );
}

#[test]
fn handles_are_compared_as_exact_strings() {
    let mut broker = Broker::new();
    queue_with_one_message(&mut broker, "a");
    let handle = receive_one(&mut broker, "a");
    assert_eq!(handle, "r-1");
    for variant in [
        "r-01",
        "R-1",
        "r-1 ",
        "r-+1",
        "r-0",
        "r-",
        "r-99999999999999999999999",
    ] {
        assert_eq!(
            delete(&mut broker, "a", variant)["error"]["code"],
            "ReceiptHandleInvalid",
            "{variant}"
        );
    }
    assert_eq!(delete(&mut broker, "a", "r-1")["ok"], true);
}

#[test]
fn delete_message_removes_the_message_and_its_handle_stops_working() {
    let mut broker = Broker::new();
    queue_with_one_message(&mut broker, "a");
    let handle = receive_one(&mut broker, "a");
    ok(
        &mut broker,
        &json!({"Command": "DeleteMessage", "QueueName": "a", "ReceiptHandle": handle}).to_string(),
    );
    assert_eq!(counts(&mut broker, "a")["InFlight"], 0);
    fail(
        &mut broker,
        &json!({"Command": "DeleteMessage", "QueueName": "a", "ReceiptHandle": handle}).to_string(),
        "ReceiptHandleExpired",
    );
}

#[test]
fn a_handle_expires_when_its_visibility_timeout_ends() {
    let mut broker = Broker::new();
    queue_with_one_message(&mut broker, "a");
    let handle = receive_one(&mut broker, "a");
    ok(&mut broker, r#"{"Command": "Tick", "Seconds": 30}"#);
    fail(
        &mut broker,
        &json!({"Command": "DeleteMessage", "QueueName": "a", "ReceiptHandle": handle}).to_string(),
        "ReceiptHandleExpired",
    );
}

#[test]
fn only_the_latest_handle_of_a_message_is_usable() {
    let mut broker = Broker::new();
    queue_with_one_message(&mut broker, "a");
    let first = receive_one(&mut broker, "a");
    ok(&mut broker, r#"{"Command": "Tick", "Seconds": 30}"#);
    let second = receive_one(&mut broker, "a");
    assert_ne!(first, second);
    fail(
        &mut broker,
        &json!({"Command": "DeleteMessage", "QueueName": "a", "ReceiptHandle": first}).to_string(),
        "ReceiptHandleExpired",
    );
    ok(
        &mut broker,
        &json!({"Command": "DeleteMessage", "QueueName": "a", "ReceiptHandle": second}).to_string(),
    );
}

#[test]
fn change_message_visibility_replaces_the_end_of_the_timeout() {
    let mut broker = Broker::new();
    queue_with_one_message(&mut broker, "a");
    let handle = receive_one(&mut broker, "a");
    ok(&mut broker, r#"{"Command": "Tick", "Seconds": 20}"#);
    ok(&mut broker, &json!({"Command": "ChangeMessageVisibility", "QueueName": "a", "ReceiptHandle": handle, "VisibilityTimeout": 30}).to_string());
    ok(&mut broker, r#"{"Command": "Tick", "Seconds": 20}"#);
    assert_eq!(
        counts(&mut broker, "a")["InFlight"],
        1,
        "now 40 of the new 30 seconds from t=20 is still in flight until t=50"
    );
    ok(&mut broker, r#"{"Command": "Tick", "Seconds": 10}"#);
    assert_eq!(
        counts(&mut broker, "a"),
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn change_message_visibility_to_zero_makes_the_message_visible_and_ends_the_handle() {
    let mut broker = Broker::new();
    queue_with_one_message(&mut broker, "a");
    let handle = receive_one(&mut broker, "a");
    ok(&mut broker, &json!({"Command": "ChangeMessageVisibility", "QueueName": "a", "ReceiptHandle": handle, "VisibilityTimeout": 0}).to_string());
    assert_eq!(counts(&mut broker, "a")["Visible"], 1);
    fail(
        &mut broker,
        &json!({"Command": "DeleteMessage", "QueueName": "a", "ReceiptHandle": handle}).to_string(),
        "ReceiptHandleExpired",
    );
}

#[test]
fn neither_command_changes_the_receive_count() {
    let mut broker = Broker::new();
    queue_with_one_message(&mut broker, "a");
    let handle = receive_one(&mut broker, "a");
    ok(&mut broker, &json!({"Command": "ChangeMessageVisibility", "QueueName": "a", "ReceiptHandle": handle, "VisibilityTimeout": 0}).to_string());
    let again = ok(
        &mut broker,
        r#"{"Command": "ReceiveMessage", "QueueName": "a"}"#,
    );
    assert_eq!(again["Messages"][0]["ReceiveCount"], 2);
}

#[test]
fn a_handle_from_another_queue_is_expired_not_invalid() {
    let mut broker = Broker::new();
    queue_with_one_message(&mut broker, "a");
    queue_with_one_message(&mut broker, "b");
    let from_a = receive_one(&mut broker, "a");
    fail(
        &mut broker,
        &json!({"Command": "DeleteMessage", "QueueName": "b", "ReceiptHandle": from_a}).to_string(),
        "ReceiptHandleExpired",
    );
}

#[test]
fn a_handle_for_a_purged_message_is_expired() {
    let mut broker = Broker::new();
    queue_with_one_message(&mut broker, "a");
    let handle = receive_one(&mut broker, "a");
    ok(
        &mut broker,
        r#"{"Command": "PurgeQueue", "QueueName": "a"}"#,
    );
    fail(
        &mut broker,
        &json!({"Command": "DeleteMessage", "QueueName": "a", "ReceiptHandle": handle}).to_string(),
        "ReceiptHandleExpired",
    );
}

#[test]
fn a_handle_for_a_message_moved_to_a_dead_letter_queue_is_expired() {
    let mut broker = Broker::new();
    ok(
        &mut broker,
        r#"{"Command": "CreateQueue", "QueueName": "dead"}"#,
    );
    ok(
        &mut broker,
        r#"{"Command": "CreateQueue", "QueueName": "jobs", "Attributes": {"DeadLetterQueue": "dead", "MaxReceiveCount": 1}}"#,
    );
    ok(
        &mut broker,
        r#"{"Command": "SendMessage", "QueueName": "jobs", "Body": "x"}"#,
    );
    let handle = receive_one(&mut broker, "jobs");
    ok(&mut broker, r#"{"Command": "Tick", "Seconds": 30}"#);
    ok(
        &mut broker,
        r#"{"Command": "ReceiveMessage", "QueueName": "jobs"}"#,
    );
    fail(
        &mut broker,
        &json!({"Command": "DeleteMessage", "QueueName": "jobs", "ReceiptHandle": handle})
            .to_string(),
        "ReceiptHandleExpired",
    );
}

#[test]
fn a_handle_from_a_deleted_queue_is_expired_even_when_the_name_is_reused() {
    let mut broker = Broker::new();
    queue_with_one_message(&mut broker, "a");
    let handle = receive_one(&mut broker, "a");
    ok(
        &mut broker,
        r#"{"Command": "DeleteQueue", "QueueName": "a"}"#,
    );
    queue_with_one_message(&mut broker, "a");
    receive_one(&mut broker, "a");
    fail(
        &mut broker,
        &json!({"Command": "DeleteMessage", "QueueName": "a", "ReceiptHandle": handle}).to_string(),
        "ReceiptHandleExpired",
    );
}

#[test]
fn a_missing_queue_is_reported_before_the_handle_is_checked() {
    let mut broker = Broker::new();
    fail(
        &mut broker,
        r#"{"Command": "DeleteMessage", "QueueName": "nope", "ReceiptHandle": "junk"}"#,
        "QueueDoesNotExist",
    );
}

#[test]
fn the_parameters_of_both_commands_are_checked() {
    let mut broker = Broker::new();
    queue_with_one_message(&mut broker, "a");
    fail(
        &mut broker,
        r#"{"Command": "DeleteMessage", "QueueName": "a"}"#,
        "MissingParameter",
    );
    fail(
        &mut broker,
        r#"{"Command": "ChangeMessageVisibility", "QueueName": "a", "ReceiptHandle": "r-1"}"#,
        "MissingParameter",
    );
    fail(
        &mut broker,
        r#"{"Command": "ChangeMessageVisibility", "QueueName": "a", "ReceiptHandle": "r-1", "VisibilityTimeout": 43201}"#,
        "InvalidParameterValue",
    );
    fail(
        &mut broker,
        r#"{"Command": "DeleteMessage", "QueueName": "a", "ReceiptHandle": 1}"#,
        "InvalidParameterValue",
    );
}
