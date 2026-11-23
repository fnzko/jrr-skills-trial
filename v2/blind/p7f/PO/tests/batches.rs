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

#[test]
fn send_batch_returns_results_in_entry_order() {
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
    let counts = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}))["Counts"].clone();
    assert_eq!(counts, json!({"Visible": 1, "InFlight": 0, "Delayed": 1}));
}

#[test]
fn failed_entries_do_not_stop_the_others_or_use_up_ids() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({"MaxMessageBytes": 3}));
    let r = send_batch(
        &mut h,
        "q",
        json!([
            {"Id": "ok1", "Body": "abc"},
            {"Id": "big", "Body": "abcd"},
            {"Id": "nobody"},
            {"Id": "grp", "Body": "x", "GroupId": "g"},
            {"Id": "nullbody", "Body": null, "Extra": 1},
            {"Id": "ok3", "Body": "z"},
        ]),
    );
    assert_eq!(r["ok"], true);
    assert_eq!(
        r["Successful"],
        json!([{"Id": "ok1", "MessageId": "m-1"}, {"Id": "ok3", "MessageId": "m-2"}])
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
            ("big", "InvalidParameterValue"),
            ("nobody", "MissingParameter"),
            ("grp", "UnsupportedOperation"),
            ("nullbody", "MissingParameter"),
        ]
    );
    assert!(r["Failed"][0]["error"]["message"].is_string());
}

#[test]
fn entry_failures_follow_single_command_checks() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let r = send_batch(
        &mut h,
        "q",
        json!([{"Id": "dedup", "Body": "x", "DeduplicationId": "d"},
               {"Id": "long", "Body": "x".repeat(1025)},
               {"Id": "empty", "Body": ""},
               {"Id": "num", "Body": 5}]),
    );
    let codes: Vec<&str> = r["Failed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["error"]["code"].as_str().unwrap())
        .collect();
    assert_eq!(
        codes,
        vec![
            "UnsupportedOperation",
            "InvalidParameterValue",
            "InvalidParameterValue",
            "InvalidParameterValue"
        ]
    );
    assert_eq!(r["Successful"], json!([]));
}

#[test]
fn fifo_entries_see_earlier_entries_deduplication_and_sequence() {
    let mut h = Harness::new();
    create(&mut h, "q.fifo", json!({"Fifo": true}));
    let r = send_batch(
        &mut h,
        "q.fifo",
        json!([
            {"Id": "a", "Body": "x", "GroupId": "g", "DeduplicationId": "d"},
            {"Id": "b", "Body": "y", "GroupId": "g", "DeduplicationId": "d"},
            {"Id": "c", "Body": "z", "GroupId": "g", "DeduplicationId": "e"},
            {"Id": "d", "Body": "w", "GroupId": "g"},
            {"Id": "e", "Body": "w", "GroupId": "g", "DelaySeconds": 1},
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
    assert_eq!(r["Failed"][0]["error"]["code"], "MissingParameter", "{r}");
    assert_eq!(r["Failed"][1]["error"]["code"], "UnsupportedOperation");
    assert_eq!(r["Failed"].as_array().unwrap().len(), 2);
}

#[test]
fn message_limit_applies_inside_a_batch() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    for chunk in 0..10_000 {
        let entries: Vec<Value> = (0..10)
            .map(|i| json!({"Id": format!("e{i}"), "Body": "x"}))
            .collect();
        let r = send_batch(&mut h, "q", json!(entries));
        assert_eq!(r["Failed"], json!([]), "chunk {chunk}");
    }
    let r = send_batch(&mut h, "q", json!([{"Id": "a", "Body": "x"}]));
    assert_eq!(r["Failed"][0]["error"]["code"], "LimitExceeded");
    assert_eq!(r["Successful"], json!([]));
}

#[test]
fn request_level_rules_in_order() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let long_id = "i".repeat(81);
    let eleven: Vec<Value> = (0..11)
        .map(|i| json!({"Id": format!("e{i}"), "Body": "x"}))
        .collect();
    let cases = [
        (json!([]), "EmptyBatchRequest"),
        (json!(eleven), "TooManyEntriesInBatchRequest"),
        (json!([5]), "InvalidBatchEntryId"),
        (json!([{"Body": "x"}]), "InvalidBatchEntryId"),
        (json!([{"Id": null, "Body": "x"}]), "InvalidBatchEntryId"),
        (json!([{"Id": 1, "Body": "x"}]), "InvalidBatchEntryId"),
        (json!([{"Id": "", "Body": "x"}]), "InvalidBatchEntryId"),
        (json!([{"Id": "a b", "Body": "x"}]), "InvalidBatchEntryId"),
        (json!([{"Id": long_id, "Body": "x"}]), "InvalidBatchEntryId"),
        (
            json!([{"Id": "a", "Body": "x"}, {"Id": "a", "Body": "y"}]),
            "BatchEntryIdsNotDistinct",
        ),
        // An invalid id is reported before duplicates.
        (
            json!([{"Id": "a"}, {"Id": "a"}, {"Id": "!"}]),
            "InvalidBatchEntryId",
        ),
        (json!({}), "InvalidParameterValue"),
        (json!("x"), "InvalidParameterValue"),
    ];
    for (entries, code) in cases {
        let r = send_batch(&mut h, "q", entries.clone());
        assert_eq!(error_code(&r), code, "{entries}");
        let r = delete_batch(&mut h, "q", entries.clone());
        assert_eq!(error_code(&r), code, "{entries}");
    }
    // Nothing was sent by any failed request.
    let counts = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}))["Counts"].clone();
    assert_eq!(counts, json!({"Visible": 0, "InFlight": 0, "Delayed": 0}));
    assert_eq!(
        h.err(json!({"Command": "SendMessageBatch", "Entries": []})),
        "MissingParameter"
    );
    assert_eq!(
        h.err(json!({"Command": "DeleteMessageBatch", "QueueName": "q"})),
        "MissingParameter"
    );
    assert_eq!(
        h.err(json!({"Command": "SendMessageBatch", "QueueName": "nope", "Entries": []})),
        "QueueDoesNotExist"
    );
    assert_eq!(
        h.err(json!({"Command": "DeleteMessageBatch", "QueueName": 5, "Entries": []})),
        "InvalidParameterValue"
    );
}

#[test]
fn ten_entries_are_allowed() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let entries: Vec<Value> = (0..10)
        .map(|i| json!({"Id": format!("e{i}"), "Body": "x"}))
        .collect();
    let r = send_batch(&mut h, "q", json!(entries));
    assert_eq!(r["Successful"].as_array().unwrap().len(), 10);
}

#[test]
fn delete_batch_handles_each_receipt_handle() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    create(&mut h, "other", json!({}));
    send_batch(
        &mut h,
        "q",
        json!([{"Id": "a", "Body": "1"}, {"Id": "b", "Body": "2"}]),
    );
    h.ok(json!({"Command": "SendMessage", "QueueName": "other", "Body": "o"}));
    h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 2})); // r-1, r-2
    h.ok(json!({"Command": "ReceiveMessage", "QueueName": "other"})); // r-3
    let r = delete_batch(
        &mut h,
        "q",
        json!([
            {"Id": "x1", "ReceiptHandle": "r-1"},
            {"Id": "x2", "ReceiptHandle": "r-1"},
            {"Id": "x3", "ReceiptHandle": "r-3"},
            {"Id": "x4", "ReceiptHandle": "r-99"},
            {"Id": "x5"},
            {"Id": "x6", "ReceiptHandle": 7},
            {"Id": "x7", "ReceiptHandle": "r-2"},
        ]),
    );
    assert_eq!(
        r,
        json!({"ok": true,
        "Successful": [{"Id": "x1"}, {"Id": "x7"}],
        "Failed": [
            {"Id": "x2", "error": {"code": "ReceiptHandleExpired",
                                   "message": r["Failed"][0]["error"]["message"]}},
            {"Id": "x3", "error": {"code": "ReceiptHandleExpired",
                                   "message": r["Failed"][1]["error"]["message"]}},
            {"Id": "x4", "error": {"code": "ReceiptHandleInvalid",
                                   "message": r["Failed"][2]["error"]["message"]}},
            {"Id": "x5", "error": {"code": "MissingParameter",
                                   "message": r["Failed"][3]["error"]["message"]}},
            {"Id": "x6", "error": {"code": "InvalidParameterValue",
                                   "message": r["Failed"][4]["error"]["message"]}},
        ]})
    );
    let counts = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}))["Counts"].clone();
    assert_eq!(counts, json!({"Visible": 0, "InFlight": 0, "Delayed": 0}));
}
