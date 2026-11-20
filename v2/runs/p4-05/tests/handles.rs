mod common;

use common::{assert_error, assert_ok, last, session};
use serde_json::{json, Value};

const STANDARD: &str = r#"{"Command": "CreateQueue", "QueueName": "jobs"}"#;

fn send(queue: &str, body: &str) -> String {
    json!({"Command": "SendMessage", "QueueName": queue, "Body": body}).to_string()
}

fn receive(queue: &str) -> String {
    json!({"Command": "ReceiveMessage", "QueueName": queue}).to_string()
}

fn tick(seconds: u32) -> String {
    json!({"Command": "Tick", "Seconds": seconds}).to_string()
}

fn delete(queue: &str, receipt: &str) -> String {
    json!({"Command": "DeleteMessage", "QueueName": queue, "ReceiptHandle": receipt}).to_string()
}

fn change(queue: &str, receipt: &str, visibility_timeout: u32) -> String {
    json!({
        "Command": "ChangeMessageVisibility",
        "QueueName": queue,
        "ReceiptHandle": receipt,
        "VisibilityTimeout": visibility_timeout,
    })
    .to_string()
}

fn counts(queue: &str) -> String {
    json!({"Command": "GetQueueAttributes", "QueueName": queue}).to_string()
}

/// The bodies of the messages in a ReceiveMessage response.
fn bodies(response: &Value) -> Vec<&str> {
    response["Messages"]
        .as_array()
        .expect("Messages is an array")
        .iter()
        .map(|message| message["Body"].as_str().unwrap())
        .collect()
}

#[test]
fn deleting_a_delivered_message_removes_it_for_good() {
    let responses = session(&[
        STANDARD,
        &send("jobs", "a"),
        &receive("jobs"),
        &delete("jobs", "r-1"),
        &tick(30),
        &receive("jobs"),
        &counts("jobs"),
    ]);

    assert_ok(&responses[3]);
    assert_eq!(responses[5], json!({"ok": true, "Messages": []}));
    assert_eq!(
        responses[6]["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn handles_that_were_never_issued_are_invalid() {
    let never_issued = [
        "r-1",
        "r-2",
        "r-0",
        "R-1",
        "r-01",
        "r-1 ",
        " r-1",
        "r-+1",
        "r-",
        "r--1",
        "r-99999999999999999999999",
        "x",
        "",
    ];
    let mut lines = vec![STANDARD.to_owned(), send("jobs", "a")];
    lines.extend(never_issued.iter().map(|receipt| delete("jobs", receipt)));
    let refs: Vec<&str> = lines.iter().map(String::as_str).collect();

    let responses = session(&refs);

    for response in &responses[2..] {
        assert_error(response, "ReceiptHandleInvalid");
    }
}

#[test]
fn a_handle_stops_working_once_its_visibility_timeout_has_ended() {
    let responses = session(&[
        STANDARD,
        &send("jobs", "a"),
        &receive("jobs"),
        &tick(29),
        &delete("jobs", "r-1"),
    ]);
    assert_ok(&responses[4]);

    let responses = session(&[
        STANDARD,
        &send("jobs", "a"),
        &receive("jobs"),
        &tick(30),
        &delete("jobs", "r-1"),
    ]);
    assert_error(&responses[4], "ReceiptHandleExpired");
}

#[test]
fn only_the_latest_handle_of_a_redelivered_message_works() {
    let responses = session(&[
        STANDARD,
        &send("jobs", "a"),
        &receive("jobs"),
        &tick(30),
        &receive("jobs"),
        &delete("jobs", "r-1"),
        &change("jobs", "r-1", 5),
        &delete("jobs", "r-2"),
    ]);

    assert_eq!(responses[4]["Messages"][0]["ReceiptHandle"], "r-2");
    assert_error(&responses[5], "ReceiptHandleExpired");
    assert_error(&responses[6], "ReceiptHandleExpired");
    assert_ok(&responses[7]);
}

#[test]
fn handle_of_a_message_in_another_queue_is_expired_there() {
    let responses = session(&[
        STANDARD,
        r#"{"Command": "CreateQueue", "QueueName": "other"}"#,
        &send("jobs", "a"),
        &receive("jobs"),
        &delete("other", "r-1"),
    ]);

    assert_error(&responses[4], "ReceiptHandleExpired");
}

#[test]
fn handle_is_expired_after_a_purge() {
    let responses = session(&[
        STANDARD,
        &send("jobs", "a"),
        &receive("jobs"),
        r#"{"Command": "PurgeQueue", "QueueName": "jobs"}"#,
        &delete("jobs", "r-1"),
    ]);

    assert_error(&responses[4], "ReceiptHandleExpired");
}

#[test]
fn handle_is_expired_after_its_queue_is_deleted_and_recreated() {
    let responses = session(&[
        STANDARD,
        &send("jobs", "a"),
        &receive("jobs"),
        r#"{"Command": "DeleteQueue", "QueueName": "jobs"}"#,
        STANDARD,
        &send("jobs", "b"),
        &delete("jobs", "r-1"),
    ]);

    assert_error(&responses[6], "ReceiptHandleExpired");
}

#[test]
fn handle_is_expired_once_the_message_has_passed_retention() {
    let responses = session(&[
        r#"{"Command": "CreateQueue", "QueueName": "jobs", "Attributes": {"RetentionSeconds": 60}}"#,
        &send("jobs", "a"),
        &receive("jobs"),
        &tick(60),
        &delete("jobs", "r-1"),
    ]);

    assert_error(&responses[4], "ReceiptHandleExpired");
}

#[test]
fn missing_queue_is_reported_before_the_handle_is_checked() {
    let response = last(&[&delete("nope", "r-99")]);

    assert_error(&response, "QueueDoesNotExist");
}

#[test]
fn delete_message_requires_both_parameters_and_string_values() {
    let responses = session(&[
        STANDARD,
        r#"{"Command": "DeleteMessage", "QueueName": "jobs"}"#,
        r#"{"Command": "DeleteMessage", "ReceiptHandle": "r-1"}"#,
        r#"{"Command": "DeleteMessage", "QueueName": "jobs", "ReceiptHandle": 1}"#,
    ]);

    assert_error(&responses[1], "MissingParameter");
    assert_error(&responses[2], "MissingParameter");
    assert_error(&responses[3], "InvalidParameterValue");
}

#[test]
fn deleting_a_message_in_flight_makes_its_group_available_again() {
    let responses = session(&[
        r#"{"Command": "CreateQueue", "QueueName": "f.fifo", "Attributes": {"Fifo": true, "ContentBasedDeduplication": true}}"#,
        r#"{"Command": "SendMessage", "QueueName": "f.fifo", "Body": "a1", "GroupId": "g"}"#,
        r#"{"Command": "SendMessage", "QueueName": "f.fifo", "Body": "a2", "GroupId": "g"}"#,
        &receive("f.fifo"),
        &delete("f.fifo", "r-1"),
        &receive("f.fifo"),
    ]);

    assert_eq!(bodies(&responses[3]), ["a1"]);
    assert_eq!(bodies(&responses[5]), ["a2"]);
}

#[test]
fn change_visibility_to_zero_makes_the_message_visible_at_once() {
    let responses = session(&[
        STANDARD,
        &send("jobs", "a"),
        &receive("jobs"),
        &change("jobs", "r-1", 0),
        &receive("jobs"),
    ]);

    assert_ok(&responses[3]);
    assert_eq!(
        responses[4]["Messages"],
        json!([{
            "MessageId": "m-1",
            "ReceiptHandle": "r-2",
            "Body": "a",
            "ReceiveCount": 2,
            "SentAt": 0,
            "FirstReceivedAt": 0,
        }])
    );
}

#[test]
fn change_visibility_sets_a_new_end_counted_from_the_time_of_the_change() {
    // Received at t=10 (in flight to t=40), then changed at t=15 to 100 seconds:
    // the new end is t=115, not t=110 (received time + 100).
    let responses = session(&[
        STANDARD,
        &send("jobs", "a"),
        &tick(10),
        &receive("jobs"),
        &tick(5),
        &change("jobs", "r-1", 100),
        &tick(99),
        &receive("jobs"),
        &tick(1),
        &receive("jobs"),
    ]);

    assert_ok(&responses[5]);
    assert_eq!(responses[7], json!({"ok": true, "Messages": []}));
    assert_eq!(responses[9]["Messages"][0]["ReceiptHandle"], "r-2");
}

#[test]
fn change_visibility_can_shorten_the_timeout() {
    let responses = session(&[
        STANDARD,
        &send("jobs", "a"),
        &receive("jobs"),
        &tick(10),
        &change("jobs", "r-1", 5),
        &tick(5),
        &receive("jobs"),
    ]);

    assert_eq!(responses[6]["Messages"][0]["ReceiptHandle"], "r-2");
}

#[test]
fn change_visibility_on_an_expired_handle_fails() {
    let responses = session(&[
        STANDARD,
        &send("jobs", "a"),
        &receive("jobs"),
        &tick(30),
        &change("jobs", "r-1", 5),
    ]);

    assert_error(&responses[4], "ReceiptHandleExpired");
}

#[test]
fn change_visibility_validates_its_parameters_in_order() {
    let responses = session(&[
        STANDARD,
        &send("jobs", "a"),
        &receive("jobs"),
        r#"{"Command": "ChangeMessageVisibility", "QueueName": "jobs", "ReceiptHandle": "r-1"}"#,
        r#"{"Command": "ChangeMessageVisibility", "QueueName": "jobs", "ReceiptHandle": "r-1", "VisibilityTimeout": 43201}"#,
        r#"{"Command": "ChangeMessageVisibility", "QueueName": "jobs", "ReceiptHandle": "r-1", "VisibilityTimeout": "5"}"#,
        &change("jobs", "r-9", 5),
        &change("nope", "r-1", 5),
    ]);

    assert_error(&responses[3], "MissingParameter");
    assert_error(&responses[4], "InvalidParameterValue");
    assert_error(&responses[5], "InvalidParameterValue");
    assert_error(&responses[6], "ReceiptHandleInvalid");
    assert_error(&responses[7], "QueueDoesNotExist");
}

#[test]
fn change_visibility_does_not_reset_the_receive_count() {
    let responses = session(&[
        STANDARD,
        &send("jobs", "a"),
        &receive("jobs"),
        &change("jobs", "r-1", 0),
        &receive("jobs"),
        &change("jobs", "r-2", 50),
        &tick(49),
        &receive("jobs"),
        &tick(1),
        &receive("jobs"),
    ]);

    assert_eq!(responses[4]["Messages"][0]["ReceiveCount"], 2);
    assert_eq!(responses[7], json!({"ok": true, "Messages": []}));
    assert_eq!(responses[9]["Messages"][0]["ReceiveCount"], 3);
}
