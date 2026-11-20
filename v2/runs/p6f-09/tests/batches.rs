mod common;

use common::Harness;
use serde_json::{Value, json};

fn create(h: &mut Harness, name: &str, attributes: Value) {
    h.ok(json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attributes}));
}

fn send_batch(h: &mut Harness, queue: &str, entries: Value) -> Value {
    h.ok(json!({"Command": "SendMessageBatch", "QueueName": queue, "Entries": entries}))
}

#[test]
fn send_batch_assigns_ids_in_order_and_reports_failures_per_entry() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({"MaxMessageBytes": 3}));
    let r = send_batch(
        &mut h,
        "q",
        json!([
            {"Id": "a", "Body": "one"},
            {"Id": "b", "Body": "too long"},
            {"Id": "c"},
            {"Id": "d", "Body": "two", "DelaySeconds": 5, "Extra": true},
            {"Id": "e", "Body": "x", "GroupId": "g"},
        ]),
    );
    assert_eq!(
        r["Successful"],
        json!([{"Id": "a", "MessageId": "m-1"}, {"Id": "d", "MessageId": "m-2"}])
    );
    let failed: Vec<(&str, &str)> = r["Failed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            (
                f["Id"].as_str().unwrap(),
                f["error"]["code"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        failed,
        [
            ("b", "InvalidParameterValue"),
            ("c", "MissingParameter"),
            ("e", "UnsupportedOperation"),
        ]
    );
    assert!(r["Failed"][0]["error"]["message"].is_string());
    let counts = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}))["Counts"].clone();
    assert_eq!(counts, json!({"Visible": 1, "InFlight": 0, "Delayed": 1}));
}

#[test]
fn send_batch_on_fifo_sees_earlier_entries_for_deduplication() {
    let mut h = Harness::new();
    create(&mut h, "q.fifo", json!({"Fifo": true}));
    let r = send_batch(
        &mut h,
        "q.fifo",
        json!([
            {"Id": "a", "Body": "x", "GroupId": "g", "DeduplicationId": "d"},
            {"Id": "b", "Body": "y", "GroupId": "g", "DeduplicationId": "d"},
            {"Id": "c", "Body": "z", "GroupId": "g", "DeduplicationId": "e"},
        ]),
    );
    assert_eq!(
        r["Successful"],
        json!([
            {"Id": "a", "MessageId": "m-1", "SequenceNumber": 1},
            {"Id": "b", "MessageId": "m-1", "SequenceNumber": 1},
            {"Id": "c", "MessageId": "m-2", "SequenceNumber": 2},
        ])
    );
    assert_eq!(r["Failed"], json!([]));
}

#[test]
fn send_batch_stops_at_the_message_limit_per_entry_only() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    for _ in 0..10_000 {
        let entries: Vec<Value> = (0..10)
            .map(|i| json!({"Id": format!("e{i}"), "Body": "b"}))
            .collect();
        let r = send_batch(&mut h, "q", json!(entries));
        assert_eq!(r["Successful"].as_array().unwrap().len(), 10);
    }
    let r = send_batch(&mut h, "q", json!([{"Id": "a", "Body": "b"}]));
    assert_eq!(r["Successful"], json!([]));
    assert_eq!(r["Failed"][0]["error"]["code"], "LimitExceeded");
}

#[test]
fn batch_level_rules_fail_the_whole_request() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let too_many: Vec<Value> = (0..11)
        .map(|i| json!({"Id": format!("e{i}"), "Body": "b"}))
        .collect();
    let long_id = "x".repeat(81);
    let cases = [
        (json!([]), "EmptyBatchRequest"),
        (json!(too_many), "TooManyEntriesInBatchRequest"),
        (json!(["a"]), "InvalidBatchEntryId"),
        (json!([{"Body": "b"}]), "InvalidBatchEntryId"),
        (json!([{"Id": null, "Body": "b"}]), "InvalidBatchEntryId"),
        (json!([{"Id": 1, "Body": "b"}]), "InvalidBatchEntryId"),
        (json!([{"Id": "", "Body": "b"}]), "InvalidBatchEntryId"),
        (json!([{"Id": "a b", "Body": "b"}]), "InvalidBatchEntryId"),
        (json!([{"Id": long_id, "Body": "b"}]), "InvalidBatchEntryId"),
        (
            json!([{"Id": "a", "Body": "b"}, {"Id": "a", "Body": "c"}]),
            "BatchEntryIdsNotDistinct",
        ),
        (
            json!([{"Id": "a", "Body": "b"}, {"Id": "A", "Body": "c"}, "x"]),
            "InvalidBatchEntryId",
        ),
        (json!("nope"), "InvalidParameterValue"),
    ];
    for (entries, code) in cases {
        let got =
            h.err(json!({"Command": "SendMessageBatch", "QueueName": "q", "Entries": entries}));
        assert_eq!(got, code, "{entries}");
    }
    let count = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}))["Counts"].clone();
    assert_eq!(count, json!({"Visible": 0, "InFlight": 0, "Delayed": 0}));
}

#[test]
fn batch_checks_required_then_queue_before_entries() {
    let mut h = Harness::new();
    assert_eq!(
        h.err(json!({"Command": "SendMessageBatch", "QueueName": "q"})),
        "MissingParameter"
    );
    assert_eq!(
        h.err(json!({"Command": "DeleteMessageBatch", "Entries": []})),
        "MissingParameter"
    );
    assert_eq!(
        h.err(json!({"Command": "DeleteMessageBatch", "QueueName": "q", "Entries": []})),
        "QueueDoesNotExist"
    );
}

#[test]
fn delete_batch_removes_messages_and_reports_bad_handles() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    send_batch(
        &mut h,
        "q",
        json!([{"Id": "a", "Body": "1"}, {"Id": "b", "Body": "2"}]),
    );
    let r = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 2}));
    let handles: Vec<Value> = r["Messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["ReceiptHandle"].clone())
        .collect();
    let r = h.ok(
        json!({"Command": "DeleteMessageBatch", "QueueName": "q", "Entries": [
            {"Id": "x", "ReceiptHandle": handles[0]},
            {"Id": "y", "ReceiptHandle": "r-99"},
            {"Id": "z", "ReceiptHandle": handles[0]},
            {"Id": "w"},
        ]}),
    );
    assert_eq!(r["Successful"], json!([{"Id": "x"}]));
    let codes: Vec<(&str, &str)> = r["Failed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            (
                f["Id"].as_str().unwrap(),
                f["error"]["code"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        codes,
        [
            ("y", "ReceiptHandleInvalid"),
            ("z", "ReceiptHandleExpired"),
            ("w", "MissingParameter")
        ]
    );
    let counts = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}))["Counts"].clone();
    assert_eq!(counts, json!({"Visible": 0, "InFlight": 1, "Delayed": 0}));
}

#[test]
fn entry_queue_name_is_ignored() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let r = send_batch(
        &mut h,
        "q",
        json!([{"Id": "a", "Body": "1", "QueueName": "other"}]),
    );
    assert_eq!(r["Successful"], json!([{"Id": "a", "MessageId": "m-1"}]));
}
