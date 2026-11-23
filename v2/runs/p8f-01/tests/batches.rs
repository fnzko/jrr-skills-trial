mod common;

use common::Harness;
use serde_json::{Value, json};

fn create(h: &mut Harness, name: &str, attributes: Value) {
    h.ok(json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attributes}));
}

fn send_batch(h: &mut Harness, queue: &str, entries: Value) -> Value {
    h.run(json!({"Command": "SendMessageBatch", "QueueName": queue, "Entries": entries}))
}

fn delete_batch(h: &mut Harness, queue: &str, entries: Value) -> Value {
    h.run(json!({"Command": "DeleteMessageBatch", "QueueName": queue, "Entries": entries}))
}

fn code(h: &mut Harness, command: &str, queue: &str, entries: Value) -> String {
    h.err(json!({"Command": command, "QueueName": queue, "Entries": entries}))
}

#[test]
fn send_batch_results_in_entry_order() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let r = send_batch(
        &mut h,
        "q",
        json!([{"Id": "a", "Body": "1"}, {"Id": "b", "Body": "2", "DelaySeconds": 5}]),
    );
    assert_eq!(
        r,
        json!({"ok": true, "Failed": [], "Successful": [
            {"Id": "a", "MessageId": "m-1"}, {"Id": "b", "MessageId": "m-2"}]})
    );
    let counts = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}))["Counts"].clone();
    assert_eq!(counts, json!({"Visible": 1, "InFlight": 0, "Delayed": 1}));
}

#[test]
fn failed_entries_do_not_stop_others_or_use_ids() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({"MaxMessageBytes": 3}));
    let r = send_batch(
        &mut h,
        "q",
        json!([
            {"Id": "a", "Body": "toolong"},
            {"Id": "b"},
            {"Id": "c", "Body": 5},
            {"Id": "d", "Body": "ok", "GroupId": "g"},
            {"Id": "e", "Body": null},
            {"Id": "f", "Body": "ok"},
            {"Id": "g", "Body": ""},
        ]),
    );
    assert_eq!(r["ok"], true);
    assert_eq!(r["Successful"], json!([{"Id": "f", "MessageId": "m-1"}]));
    let failed: Vec<(&str, &str)> = r["Failed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            assert!(f["error"]["message"].is_string());
            assert_eq!(f.as_object().unwrap().len(), 2);
            (f["Id"].as_str().unwrap(), f["error"]["code"].as_str().unwrap())
        })
        .collect();
    assert_eq!(
        failed,
        [
            ("a", "InvalidParameterValue"),
            ("b", "MissingParameter"),
            ("c", "InvalidParameterValue"),
            ("d", "UnsupportedOperation"),
            ("e", "MissingParameter"),
            ("g", "InvalidParameterValue"),
        ]
    );
}

#[test]
fn fifo_entries_see_earlier_entries() {
    let mut h = Harness::new();
    create(&mut h, "f.fifo", json!({"Fifo": true, "ContentBasedDeduplication": true}));
    let r = send_batch(
        &mut h,
        "f.fifo",
        json!([
            {"Id": "1", "Body": "x", "GroupId": "g"},
            {"Id": "2", "Body": "x", "GroupId": "g"},
            {"Id": "3", "Body": "y", "GroupId": "g", "DeduplicationId": "d"},
            {"Id": "4", "Body": "z"},
            {"Id": "5", "Body": "w", "GroupId": "g", "DelaySeconds": 1},
        ]),
    );
    assert_eq!(
        r["Successful"],
        json!([
            {"Id": "1", "MessageId": "m-1", "SequenceNumber": 1},
            {"Id": "2", "MessageId": "m-1", "SequenceNumber": 1},
            {"Id": "3", "MessageId": "m-2", "SequenceNumber": 2},
        ])
    );
    assert_eq!(r["Failed"][0]["error"]["code"], "MissingParameter");
    assert_eq!(r["Failed"][1]["error"]["code"], "UnsupportedOperation");
}

#[test]
fn delete_batch_uses_handles_in_order() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    create(&mut h, "other", json!({}));
    h.ok(json!({"Command": "SendMessage", "QueueName": "q", "Body": "a"}));
    h.ok(json!({"Command": "SendMessage", "QueueName": "q", "Body": "b"}));
    h.ok(json!({"Command": "SendMessage", "QueueName": "other", "Body": "c"}));
    h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 2}));
    h.ok(json!({"Command": "ReceiveMessage", "QueueName": "other"}));
    let r = delete_batch(
        &mut h,
        "q",
        json!([
            {"Id": "a", "ReceiptHandle": "r-1"},
            {"Id": "b", "ReceiptHandle": "r-1"},
            {"Id": "c", "ReceiptHandle": "r-3"},
            {"Id": "d", "ReceiptHandle": "r-9"},
            {"Id": "e"},
            {"Id": "f", "ReceiptHandle": 1},
            {"Id": "g", "ReceiptHandle": "r-2"},
        ]),
    );
    assert_eq!(r["Successful"], json!([{"Id": "a"}, {"Id": "g"}]));
    let codes: Vec<&str> = r["Failed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["error"]["code"].as_str().unwrap())
        .collect();
    assert_eq!(
        codes,
        [
            "ReceiptHandleExpired",
            "ReceiptHandleExpired",
            "ReceiptHandleInvalid",
            "MissingParameter",
            "InvalidParameterValue"
        ]
    );
    let counts = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}))["Counts"].clone();
    assert_eq!(counts, json!({"Visible": 0, "InFlight": 0, "Delayed": 0}));
}

#[test]
fn request_checks_fail_the_whole_batch_and_do_nothing() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let ok_entry = json!({"Id": "a", "Body": "x"});
    let eleven: Vec<Value> = (0..11).map(|i| json!({"Id": format!("i{i}"), "Body": "x"})).collect();
    let cases = [
        (json!([]), "EmptyBatchRequest"),
        (Value::Array(eleven), "TooManyEntriesInBatchRequest"),
        (json!([ok_entry, 5]), "InvalidBatchEntryId"),
        (json!([ok_entry, {"Body": "x"}]), "InvalidBatchEntryId"),
        (json!([ok_entry, {"Id": null}]), "InvalidBatchEntryId"),
        (json!([ok_entry, {"Id": 1}]), "InvalidBatchEntryId"),
        (json!([ok_entry, {"Id": ""}]), "InvalidBatchEntryId"),
        (json!([ok_entry, {"Id": "a b"}]), "InvalidBatchEntryId"),
        (json!([ok_entry, {"Id": "x".repeat(81)}]), "InvalidBatchEntryId"),
        (json!([ok_entry, ok_entry]), "BatchEntryIdsNotDistinct"),
        // Rule 3 is checked on every entry before rule 4.
        (json!([ok_entry, ok_entry, 5]), "InvalidBatchEntryId"),
    ];
    for (entries, expected) in cases {
        assert_eq!(code(&mut h, "SendMessageBatch", "q", entries.clone()), expected);
        assert_eq!(code(&mut h, "DeleteMessageBatch", "q", entries), expected);
    }
    let r = send_batch(&mut h, "q", json!([{"Id": "x".repeat(80), "Body": "x"}]));
    assert_eq!(r["Successful"], json!([{"Id": "x".repeat(80), "MessageId": "m-1"}]));
}

#[test]
fn batch_checks_follow_the_check_order() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    for command in ["SendMessageBatch", "DeleteMessageBatch"] {
        assert_eq!(h.err(json!({"Command": command, "QueueName": "q"})), "MissingParameter");
        assert_eq!(h.err(json!({"Command": command, "Entries": []})), "MissingParameter");
        assert_eq!(
            h.err(json!({"Command": command, "QueueName": "q", "Entries": {}})),
            "InvalidParameterValue"
        );
        assert_eq!(
            h.err(json!({"Command": command, "QueueName": 1, "Entries": []})),
            "InvalidParameterValue"
        );
        // Queue lookup comes before the batch rules.
        assert_eq!(code(&mut h, command, "none", json!([])), "QueueDoesNotExist");
    }
}

#[test]
fn send_batch_entries_fail_individually_at_the_message_limit() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    // Entries beyond the limit fail individually; the batch itself succeeds.
    // 100000 messages in batches of ten.
    let entries: Vec<Value> = (0..10).map(|i| json!({"Id": format!("i{i}"), "Body": "x"})).collect();
    for _ in 0..10_000 {
        let r = send_batch(&mut h, "q", Value::Array(entries.clone()));
        assert_eq!(r["Failed"], json!([]));
    }
    let r = send_batch(&mut h, "q", json!([{"Id": "a", "Body": "x"}]));
    assert_eq!(r["ok"], true);
    assert_eq!(r["Successful"], json!([]));
    assert_eq!(r["Failed"][0]["error"]["code"], "LimitExceeded");
}
