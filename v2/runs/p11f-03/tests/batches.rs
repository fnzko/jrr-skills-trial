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

fn error_code(response: &Value) -> &str {
    assert_eq!(response["ok"], false, "{response}");
    response["error"]["code"].as_str().unwrap()
}

fn counts(h: &mut Harness, queue: &str) -> Value {
    h.ok(json!({"Command": "GetQueueAttributes", "QueueName": queue}))["Counts"].clone()
}

#[test]
fn send_batch_accepts_entries_in_order() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let r = send_batch(
        &mut h,
        "q",
        json!([{"Id": "a", "Body": "one"}, {"Id": "b", "Body": "two", "DelaySeconds": 5}]),
    );
    assert_eq!(
        r,
        json!({"ok": true, "Failed": [],
               "Successful": [{"Id": "a", "MessageId": "m-1"}, {"Id": "b", "MessageId": "m-2"}]})
    );
    assert_eq!(
        counts(&mut h, "q"),
        json!({"Visible": 1, "InFlight": 0, "Delayed": 1})
    );
}

#[test]
fn failed_entries_do_not_stop_the_others_or_use_message_ids() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({"MaxMessageBytes": 3}));
    let r = send_batch(
        &mut h,
        "q",
        json!([
            {"Id": "ok1", "Body": "abc"},
            {"Id": "toolong", "Body": "abcd"},
            {"Id": "nobody"},
            {"Id": "badtype", "Body": 5},
            {"Id": "group", "Body": "x", "GroupId": "g"},
            {"Id": "ok2", "Body": "xyz", "QueueName": "ignored", "Extra": 1}
        ]),
    );
    assert_eq!(r["ok"], true);
    assert_eq!(
        r["Successful"],
        json!([{"Id": "ok1", "MessageId": "m-1"}, {"Id": "ok2", "MessageId": "m-2"}])
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
        vec![
            ("toolong", "InvalidParameterValue"),
            ("nobody", "MissingParameter"),
            ("badtype", "InvalidParameterValue"),
            ("group", "UnsupportedOperation"),
        ]
    );
    assert!(r["Failed"][0]["error"]["message"].is_string());
}

#[test]
fn all_entries_failing_is_still_a_success() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let r = send_batch(&mut h, "q", json!([{"Id": "a"}]));
    assert_eq!(r["ok"], true);
    assert_eq!(r["Successful"], json!([]));
    assert_eq!(r["Failed"][0]["error"]["code"], "MissingParameter");
}

#[test]
fn fifo_entries_see_earlier_entries() {
    let mut h = Harness::new();
    create(
        &mut h,
        "q.fifo",
        json!({"Fifo": true, "ContentBasedDeduplication": true}),
    );
    let r = send_batch(
        &mut h,
        "q.fifo",
        json!([
            {"Id": "a", "Body": "same", "GroupId": "g"},
            {"Id": "b", "Body": "same", "GroupId": "g"},
            {"Id": "c", "Body": "other", "GroupId": "g"},
            {"Id": "d", "Body": "x", "GroupId": "g", "DelaySeconds": 1},
            {"Id": "e", "Body": "y"}
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
    assert_eq!(r["Failed"][0]["Id"], "d");
    assert_eq!(r["Failed"][0]["error"]["code"], "UnsupportedOperation");
    assert_eq!(r["Failed"][1]["Id"], "e");
    assert_eq!(r["Failed"][1]["error"]["code"], "MissingParameter");
}

#[test]
fn queue_limit_applies_within_a_batch() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    // 100000 messages cannot be sent cheaply by hand; fill with batches.
    for _ in 0..10_000 {
        let entries: Vec<Value> = (0..10)
            .map(|i| json!({"Id": format!("e{i}"), "Body": "x"}))
            .collect();
        let r = send_batch(&mut h, "q", Value::Array(entries));
        assert_eq!(r["Failed"], json!([]));
    }
    let r = send_batch(&mut h, "q", json!([{"Id": "a", "Body": "x"}]));
    assert_eq!(r["Failed"][0]["error"]["code"], "LimitExceeded");
    assert_eq!(r["Successful"], json!([]));
}

#[test]
fn batch_level_errors_follow_the_check_order() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let eleven: Vec<Value> = (0..11)
        .map(|i| json!({"Id": format!("e{i}"), "Body": "x"}))
        .collect();
    let cases: Vec<(Value, &str)> = vec![
        (
            json!({"Command": "SendMessageBatch", "Entries": []}),
            "MissingParameter",
        ),
        (
            json!({"Command": "SendMessageBatch", "QueueName": "q"}),
            "MissingParameter",
        ),
        (
            json!({"Command": "SendMessageBatch", "QueueName": "q", "Entries": null}),
            "MissingParameter",
        ),
        (
            json!({"Command": "SendMessageBatch", "QueueName": "q", "Entries": {}}),
            "InvalidParameterValue",
        ),
        (
            json!({"Command": "SendMessageBatch", "QueueName": "q", "Entries": "a"}),
            "InvalidParameterValue",
        ),
        (
            json!({"Command": "SendMessageBatch", "QueueName": "nope", "Entries": []}),
            "QueueDoesNotExist",
        ),
        (
            json!({"Command": "SendMessageBatch", "QueueName": "q", "Entries": []}),
            "EmptyBatchRequest",
        ),
        (
            json!({"Command": "SendMessageBatch", "QueueName": "q", "Entries": eleven}),
            "TooManyEntriesInBatchRequest",
        ),
        (
            json!({"Command": "SendMessageBatch", "QueueName": "q", "Entries": [5]}),
            "InvalidBatchEntryId",
        ),
        (
            json!({"Command": "SendMessageBatch", "QueueName": "q", "Entries": [{"Body": "x"}]}),
            "InvalidBatchEntryId",
        ),
        (
            json!({"Command": "SendMessageBatch", "QueueName": "q", "Entries": [{"Id": null, "Body": "x"}]}),
            "InvalidBatchEntryId",
        ),
        (
            json!({"Command": "SendMessageBatch", "QueueName": "q", "Entries": [{"Id": 1, "Body": "x"}]}),
            "InvalidBatchEntryId",
        ),
        (
            json!({"Command": "SendMessageBatch", "QueueName": "q", "Entries": [{"Id": "", "Body": "x"}]}),
            "InvalidBatchEntryId",
        ),
        (
            json!({"Command": "SendMessageBatch", "QueueName": "q", "Entries": [{"Id": "a b", "Body": "x"}]}),
            "InvalidBatchEntryId",
        ),
        (
            json!({"Command": "SendMessageBatch", "QueueName": "q", "Entries": [{"Id": "x".repeat(81), "Body": "x"}]}),
            "InvalidBatchEntryId",
        ),
        (
            json!({"Command": "SendMessageBatch", "QueueName": "q", "Entries": [{"Id": "a"}, {"Id": "a"}]}),
            "BatchEntryIdsNotDistinct",
        ),
        (
            json!({"Command": "SendMessageBatch", "QueueName": "q", "Entries": [{"Id": "a"}, {"Id": "A"}, {"Id": "a"}]}),
            "BatchEntryIdsNotDistinct",
        ),
        (
            json!({"Command": "DeleteMessageBatch", "QueueName": "q", "Entries": []}),
            "EmptyBatchRequest",
        ),
    ];
    for (command, code) in cases {
        let r = h.run(command.clone());
        assert_eq!(error_code(&r), code, "{command}");
    }
    // The failed requests above sent nothing.
    assert_eq!(
        counts(&mut h, "q"),
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn rule_order_too_many_before_bad_ids() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let mut entries: Vec<Value> = (0..10).map(|_| json!(5)).collect();
    entries.push(json!(5));
    let r = send_batch(&mut h, "q", Value::Array(entries));
    assert_eq!(error_code(&r), "TooManyEntriesInBatchRequest");
    let r = send_batch(&mut h, "q", json!([{"Id": "a"}, {"Id": "a"}, 7]));
    assert_eq!(error_code(&r), "InvalidBatchEntryId");
}

#[test]
fn ten_entries_with_valid_ids_are_allowed() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let entries: Vec<Value> = (0..10)
        .map(
            |i| json!({"Id": if i == 0 { "x".repeat(80) } else { format!("e-{i}_") }, "Body": "x"}),
        )
        .collect();
    let r = send_batch(&mut h, "q", Value::Array(entries));
    assert_eq!(r["Successful"].as_array().unwrap().len(), 10);
}

fn receive_handles(h: &mut Harness, queue: &str, max: u64) -> Vec<String> {
    let r = h.ok(json!({"Command": "ReceiveMessage", "QueueName": queue, "MaxMessages": max}));
    r["Messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["ReceiptHandle"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn delete_batch_reports_each_entry() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    for body in ["a", "b", "c"] {
        h.ok(json!({"Command": "SendMessage", "QueueName": "q", "Body": body}));
    }
    let handles = receive_handles(&mut h, "q", 3);
    let r = delete_batch(
        &mut h,
        "q",
        json!([
            {"Id": "d1", "ReceiptHandle": handles[0]},
            {"Id": "never", "ReceiptHandle": "r-99"},
            {"Id": "again", "ReceiptHandle": handles[0]},
            {"Id": "missing"},
            {"Id": "d2", "ReceiptHandle": handles[2]}
        ]),
    );
    assert_eq!(r["ok"], true);
    assert_eq!(r["Successful"], json!([{"Id": "d1"}, {"Id": "d2"}]));
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
        vec![
            ("never", "ReceiptHandleInvalid"),
            ("again", "ReceiptHandleExpired"),
            ("missing", "MissingParameter"),
        ]
    );
    assert_eq!(
        counts(&mut h, "q"),
        json!({"Visible": 0, "InFlight": 1, "Delayed": 0})
    );
}

#[test]
fn delete_batch_rejects_bad_handle_types_per_entry() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let r = delete_batch(&mut h, "q", json!([{"Id": "a", "ReceiptHandle": 1}]));
    assert_eq!(r["Failed"][0]["error"]["code"], "InvalidParameterValue");
    assert_eq!(r["Successful"], json!([]));
}
