mod common;

use common::Harness;
use serde_json::{Value, json};

fn create(h: &mut Harness, name: &str, attributes: Value) {
    h.ok(json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attributes}));
}

fn send_batch(queue: &str, entries: Value) -> Value {
    json!({"Command": "SendMessageBatch", "QueueName": queue, "Entries": entries})
}

fn delete_batch(queue: &str, entries: Value) -> Value {
    json!({"Command": "DeleteMessageBatch", "QueueName": queue, "Entries": entries})
}

fn error_codes(failed: &Value) -> Vec<(&str, &str)> {
    failed
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            assert!(f["error"]["message"].is_string());
            (
                f["Id"].as_str().unwrap(),
                f["error"]["code"].as_str().unwrap(),
            )
        })
        .collect()
}

#[test]
fn send_batch_assigns_ids_in_entry_order() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let r = h.ok(send_batch(
        "q",
        json!([{"Id": "a", "Body": "x"}, {"Id": "b", "Body": "y", "Extra": 1}]),
    ));
    assert_eq!(
        r,
        json!({"ok": true, "Failed": [],
               "Successful": [{"Id": "a", "MessageId": "m-1"}, {"Id": "b", "MessageId": "m-2"}]})
    );
}

#[test]
fn failed_entries_do_not_stop_others_or_use_message_ids() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({"MaxMessageBytes": 2}));
    let r = h.ok(send_batch(
        "q",
        json!([
            {"Id": "a", "Body": "toolong"},
            {"Id": "b"},
            {"Id": "c", "Body": 5},
            {"Id": "d", "Body": "ok", "GroupId": "g"},
            {"Id": "e", "Body": "ok", "DelaySeconds": 901},
            {"Id": "f", "Body": "ok", "DelaySeconds": null},
        ]),
    ));
    assert_eq!(
        error_codes(&r["Failed"]),
        [
            ("a", "InvalidParameterValue"),
            ("b", "MissingParameter"),
            ("c", "InvalidParameterValue"),
            ("d", "UnsupportedOperation"),
            ("e", "InvalidParameterValue"),
        ]
    );
    assert_eq!(r["Successful"], json!([{"Id": "f", "MessageId": "m-1"}]));
}

#[test]
fn fifo_entries_see_earlier_entries_deduplication_and_sequence() {
    let mut h = Harness::new();
    create(&mut h, "q.fifo", json!({"Fifo": true}));
    let r = h.ok(send_batch(
        "q.fifo",
        json!([
            {"Id": "a", "Body": "x", "GroupId": "g", "DeduplicationId": "d"},
            {"Id": "b", "Body": "y", "GroupId": "g", "DeduplicationId": "d"},
            {"Id": "c", "Body": "z", "GroupId": "g", "DeduplicationId": "e"},
            {"Id": "d", "Body": "z", "DeduplicationId": "f"},
        ]),
    ));
    assert_eq!(
        r["Successful"],
        json!([
            {"Id": "a", "MessageId": "m-1", "SequenceNumber": 1},
            {"Id": "b", "MessageId": "m-1", "SequenceNumber": 1},
            {"Id": "c", "MessageId": "m-2", "SequenceNumber": 2},
        ])
    );
    assert_eq!(error_codes(&r["Failed"]), [("d", "MissingParameter")]);
}

#[test]
fn batch_stops_at_the_queue_message_limit_per_entry() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    for _ in 0..10_000 {
        let entries: Vec<Value> = (0..10)
            .map(|i| json!({"Id": format!("i{i}"), "Body": "x"}))
            .collect();
        h.ok(send_batch("q", Value::Array(entries)));
    }
    let r = h.ok(send_batch(
        "q",
        json!([{"Id": "a", "Body": "x"}, {"Id": "b", "Body": "x"}]),
    ));
    assert_eq!(r["Successful"], json!([]));
    assert_eq!(
        error_codes(&r["Failed"]),
        [("a", "LimitExceeded"), ("b", "LimitExceeded")]
    );
}

#[test]
fn batch_rules_and_their_order() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let eleven: Vec<Value> = (0..11)
        .map(|i| json!({"Id": format!("i{i}"), "Body": "x"}))
        .collect();
    let ten: Vec<Value> = eleven[..10].to_vec();
    h.ok(send_batch("q", Value::Array(ten)));
    let cases = [
        (json!([]), "EmptyBatchRequest"),
        (Value::Array(eleven), "TooManyEntriesInBatchRequest"),
        (json!([5]), "InvalidBatchEntryId"),
        (json!([null]), "InvalidBatchEntryId"),
        (json!([{"Body": "x"}]), "InvalidBatchEntryId"),
        (json!([{"Id": null, "Body": "x"}]), "InvalidBatchEntryId"),
        (json!([{"Id": 1, "Body": "x"}]), "InvalidBatchEntryId"),
        (json!([{"Id": "", "Body": "x"}]), "InvalidBatchEntryId"),
        (json!([{"Id": "a b", "Body": "x"}]), "InvalidBatchEntryId"),
        (
            json!([{"Id": "a".repeat(81), "Body": "x"}]),
            "InvalidBatchEntryId",
        ),
        (
            json!([{"Id": "a", "Body": "x"}, {"Id": "a", "Body": "y"}]),
            "BatchEntryIdsNotDistinct",
        ),
        (
            json!([{"Id": "a"}, {"Id": "a"}, {"Id": "*"}]),
            "InvalidBatchEntryId",
        ),
    ];
    for (entries, code) in cases {
        assert_eq!(h.err(send_batch("q", entries.clone())), code, "{entries}");
        assert_eq!(h.err(delete_batch("q", entries.clone())), code, "{entries}");
    }
    h.ok(send_batch(
        "q",
        json!([{"Id": "a".repeat(80), "Body": "x"}]),
    ));
    let counts = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}))["Counts"].clone();
    assert_eq!(counts["Visible"], 11);
}

#[test]
fn batch_parameters_are_checked_before_the_queue_lookup_and_rules() {
    let mut h = Harness::new();
    assert_eq!(
        h.err(json!({"Command": "SendMessageBatch", "QueueName": "q"})),
        "MissingParameter"
    );
    assert_eq!(
        h.err(json!({"Command": "SendMessageBatch", "Entries": []})),
        "MissingParameter"
    );
    assert_eq!(h.err(send_batch("q", json!("x"))), "InvalidParameterValue");
    assert_eq!(h.err(send_batch("q", json!({}))), "InvalidParameterValue");
    assert_eq!(h.err(send_batch("q", json!([]))), "QueueDoesNotExist");
    assert_eq!(h.err(delete_batch("q", json!([]))), "QueueDoesNotExist");
}

#[test]
fn delete_batch_reports_each_handle() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    create(&mut h, "other", json!({}));
    h.ok(send_batch(
        "q",
        json!([{"Id": "a", "Body": "x"}, {"Id": "b", "Body": "y"}]),
    ));
    let received = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 2}));
    let first = received["Messages"][0]["ReceiptHandle"].clone();
    let second = received["Messages"][1]["ReceiptHandle"].clone();
    let r = h.ok(delete_batch(
        "q",
        json!([
            {"Id": "1", "ReceiptHandle": first},
            {"Id": "2", "ReceiptHandle": first},
            {"Id": "3", "ReceiptHandle": "r-99"},
            {"Id": "4"},
            {"Id": "5", "ReceiptHandle": 7},
            {"Id": "6", "ReceiptHandle": second},
        ]),
    ));
    assert_eq!(r["Successful"], json!([{"Id": "1"}, {"Id": "6"}]));
    assert_eq!(
        error_codes(&r["Failed"]),
        [
            ("2", "ReceiptHandleExpired"),
            ("3", "ReceiptHandleInvalid"),
            ("4", "MissingParameter"),
            ("5", "InvalidParameterValue"),
        ]
    );
    let counts = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}))["Counts"].clone();
    assert_eq!(counts, json!({"Visible": 0, "InFlight": 0, "Delayed": 0}));
}

#[test]
fn delete_batch_handle_of_another_queue_is_expired() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    create(&mut h, "other", json!({}));
    h.ok(json!({"Command": "SendMessage", "QueueName": "other", "Body": "x"}));
    let received = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "other"}));
    let handle = received["Messages"][0]["ReceiptHandle"].clone();
    let r = h.ok(delete_batch(
        "q",
        json!([{"Id": "a", "ReceiptHandle": handle}]),
    ));
    assert_eq!(error_codes(&r["Failed"]), [("a", "ReceiptHandleExpired")]);
}
