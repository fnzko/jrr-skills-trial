mod common;

use common::{error_code, run};
use serde_json::{Value, json};

const CREATE_F: &str =
    r#"{"Command": "CreateQueue", "QueueName": "f.fifo", "Attributes": {"Fifo": true}}"#;
const CREATE_CBD: &str = r#"{"Command": "CreateQueue", "QueueName": "f.fifo", "Attributes": {"Fifo": true, "ContentBasedDeduplication": true}}"#;
const RECEIVE_ALL: &str =
    r#"{"Command": "ReceiveMessage", "QueueName": "f.fifo", "MaxMessages": 10}"#;

fn send(body: &str, group: &str, dedup: &str) -> String {
    json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": body,
           "GroupId": group, "DeduplicationId": dedup})
    .to_string()
}

fn tick(seconds: u32) -> String {
    json!({"Command": "Tick", "Seconds": seconds}).to_string()
}

fn bodies(response: &Value) -> Vec<&str> {
    response["Messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["Body"].as_str().unwrap())
        .collect()
}

#[test]
fn sends_get_sequence_numbers_and_receives_report_group_and_sequence() {
    let out = run(&[
        CREATE_F,
        &send("a", "g1", "d1"),
        &send("b", "g2", "d2"),
        RECEIVE_ALL,
    ]);
    assert_eq!(
        out[1],
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
    assert_eq!(
        out[2],
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2})
    );
    assert_eq!(
        out[3]["Messages"],
        json!([
            {"MessageId": "m-1", "ReceiptHandle": "r-1", "Body": "a", "ReceiveCount": 1,
             "SentAt": 0, "FirstReceivedAt": 0, "GroupId": "g1", "SequenceNumber": 1},
            {"MessageId": "m-2", "ReceiptHandle": "r-2", "Body": "b", "ReceiveCount": 1,
             "SentAt": 0, "FirstReceivedAt": 0, "GroupId": "g2", "SequenceNumber": 2}
        ])
    );
}

#[test]
fn send_rules_in_order() {
    let out = run(&[
        CREATE_F,
        r#"{"Command": "SendMessage", "QueueName": "f.fifo", "Body": "x", "DelaySeconds": 0}"#,
        r#"{"Command": "SendMessage", "QueueName": "f.fifo", "Body": "x", "DelaySeconds": 0, "GroupId": "g", "DeduplicationId": "d"}"#,
        r#"{"Command": "SendMessage", "QueueName": "f.fifo", "Body": "x"}"#,
        r#"{"Command": "SendMessage", "QueueName": "f.fifo", "Body": "x", "DeduplicationId": "d"}"#,
        r#"{"Command": "SendMessage", "QueueName": "f.fifo", "Body": "x", "GroupId": "g"}"#,
        r#"{"Command": "SendMessage", "QueueName": "f.fifo", "Body": "x", "GroupId": "g", "DeduplicationId": "d"}"#,
    ]);
    assert_eq!(error_code(&out[1]), "UnsupportedOperation");
    assert_eq!(error_code(&out[2]), "UnsupportedOperation");
    assert_eq!(error_code(&out[3]), "MissingParameter");
    assert_eq!(error_code(&out[4]), "MissingParameter");
    assert_eq!(error_code(&out[5]), "MissingParameter");
    assert_eq!(out[6]["MessageId"], "m-1");
}

#[test]
fn a_duplicate_returns_the_original_ids_and_uses_no_numbers() {
    let out = run(&[
        CREATE_F,
        &send("a", "g1", "d"),
        &send("other body", "g2", "d"),
        &send("b", "g1", "e"),
    ]);
    assert_eq!(out[2], out[1]);
    assert_eq!(
        out[3],
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2})
    );
}

#[test]
fn the_deduplication_window_is_300_seconds_and_is_not_extended_by_duplicates() {
    let out = run(&[
        CREATE_F,
        &send("a", "g", "d"),
        &tick(299),
        &send("a", "g", "d"),
        &tick(1),
        &send("a", "g", "d"),
        &tick(299),
        &send("a", "g", "d"),
        &tick(1),
        &send("a", "g", "d"),
    ]);
    assert_eq!(out[3]["MessageId"], "m-1");
    assert_eq!(
        out[5],
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2})
    );
    assert_eq!(out[7]["MessageId"], "m-2");
    assert_eq!(
        out[9],
        json!({"ok": true, "MessageId": "m-3", "SequenceNumber": 3})
    );
}

#[test]
fn deduplication_outlives_the_message_and_survives_purge() {
    let out = run(&[
        CREATE_F,
        &send("a", "g", "d"),
        RECEIVE_ALL,
        r#"{"Command": "DeleteMessage", "QueueName": "f.fifo", "ReceiptHandle": "r-1"}"#,
        &send("a", "g", "d"),
        &send("b", "g", "e"),
        r#"{"Command": "PurgeQueue", "QueueName": "f.fifo"}"#,
        &send("b", "g", "e"),
        &send("c", "g", "f"),
    ]);
    assert_eq!(out[4], out[1]);
    assert_eq!(out[7], out[5]);
    // Sequence numbers carry on after the purge.
    assert_eq!(
        out[8],
        json!({"ok": true, "MessageId": "m-3", "SequenceNumber": 3})
    );
}

#[test]
fn deduplication_ids_are_per_queue() {
    let out = run(&[
        CREATE_F,
        r#"{"Command": "CreateQueue", "QueueName": "g.fifo", "Attributes": {"Fifo": true}}"#,
        &send("a", "g", "d"),
        r#"{"Command": "SendMessage", "QueueName": "g.fifo", "Body": "a", "GroupId": "g", "DeduplicationId": "d"}"#,
    ]);
    assert_eq!(
        out[3],
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 1})
    );
}

#[test]
fn content_based_deduplication_uses_the_body_unless_an_id_is_given() {
    let out = run(&[
        CREATE_CBD,
        r#"{"Command": "SendMessage", "QueueName": "f.fifo", "Body": "same", "GroupId": "g1"}"#,
        r#"{"Command": "SendMessage", "QueueName": "f.fifo", "Body": "same", "GroupId": "g2"}"#,
        r#"{"Command": "SendMessage", "QueueName": "f.fifo", "Body": "same", "GroupId": "g2", "DeduplicationId": "explicit"}"#,
        r#"{"Command": "SendMessage", "QueueName": "f.fifo", "Body": "different", "GroupId": "g2"}"#,
    ]);
    assert_eq!(out[2], out[1]);
    assert_eq!(
        out[3],
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2})
    );
    assert_eq!(
        out[4],
        json!({"ok": true, "MessageId": "m-3", "SequenceNumber": 3})
    );
}

#[test]
fn receive_delivers_at_most_one_message_per_group_and_keeps_group_order() {
    let out = run(&[
        CREATE_F,
        &send("a1", "a", "1"),
        &send("a2", "a", "2"),
        &send("b1", "b", "3"),
        &send("a3", "a", "4"),
        RECEIVE_ALL,
        RECEIVE_ALL,
        r#"{"Command": "DeleteMessage", "QueueName": "f.fifo", "ReceiptHandle": "r-1"}"#,
        RECEIVE_ALL,
    ]);
    assert_eq!(bodies(&out[5]), ["a1", "b1"]);
    assert_eq!(bodies(&out[6]), Vec::<&str>::new());
    assert_eq!(bodies(&out[8]), ["a2"]);
}

#[test]
fn a_group_stays_blocked_while_its_earliest_message_is_in_flight_even_after_others_are_visible() {
    let out = run(&[
        CREATE_F,
        &send("a1", "a", "1"),
        &send("a2", "a", "2"),
        r#"{"Command": "ReceiveMessage", "QueueName": "f.fifo", "VisibilityTimeout": 10}"#,
        &tick(10),
        RECEIVE_ALL,
    ]);
    // The timeout of a1 ended, so a1 is delivered again, before a2.
    assert_eq!(bodies(&out[5]), ["a1"]);
    assert_eq!(out[5]["Messages"][0]["ReceiveCount"], 2);
}
