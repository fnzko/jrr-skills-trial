mod common;

use common::Harness;
use serde_json::{Value, json};

fn create(h: &mut Harness, name: &str, attributes: Value) {
    h.ok(json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attributes}));
}

fn send_batch(h: &mut Harness, queue: &str, entries: Value) -> Value {
    h.run(json!({"Command": "SendMessageBatch", "QueueName": queue, "Entries": entries}))
}

fn receive(h: &mut Harness, queue: &str, max: u64) -> Vec<Value> {
    let r = h.ok(json!({"Command": "ReceiveMessage", "QueueName": queue, "MaxMessages": max}));
    r["Messages"].as_array().unwrap().clone()
}

#[test]
fn send_batch_returns_results_in_entry_order() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let r = send_batch(
        &mut h,
        "q",
        json!([{"Id": "b", "Body": "x"}, {"Id": "a", "Body": "y"}]),
    );
    assert_eq!(
        r,
        json!({"ok": true, "Failed": [],
               "Successful": [{"Id": "b", "MessageId": "m-1"}, {"Id": "a", "MessageId": "m-2"}]})
    );
}

#[test]
fn failed_entries_do_not_stop_the_others_or_use_ids() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({"MaxMessageBytes": 3}));
    let r = send_batch(
        &mut h,
        "q",
        json!([{"Id": "1", "Body": "ok"}, {"Id": "2"}, {"Id": "3", "Body": "toolong"},
               {"Id": "4", "Body": "x", "DelaySeconds": 901}, {"Id": "5", "Body": "z", "GroupId": "g"},
               {"Id": "6", "Body": "fine"}, {"Id": "7", "Body": "yes"}]),
    );
    assert_eq!(r["ok"], true);
    assert_eq!(
        r["Successful"],
        json!([{"Id": "1", "MessageId": "m-1"}, {"Id": "7", "MessageId": "m-2"}])
    );
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
        vec![
            ("2", "MissingParameter"),
            ("3", "InvalidParameterValue"),
            ("4", "InvalidParameterValue"),
            ("5", "UnsupportedOperation"),
            ("6", "InvalidParameterValue"),
        ]
    );
    assert!(r["Failed"][0]["error"]["message"].is_string());
}

#[test]
fn entries_see_earlier_entries_fifo_and_deduplication() {
    let mut h = Harness::new();
    create(&mut h, "q.fifo", json!({"Fifo": true}));
    let r = send_batch(
        &mut h,
        "q.fifo",
        json!([{"Id": "a", "Body": "x", "GroupId": "g", "DeduplicationId": "d"},
               {"Id": "b", "Body": "y", "GroupId": "g", "DeduplicationId": "d"},
               {"Id": "c", "Body": "z", "GroupId": "g", "DeduplicationId": "e"}]),
    );
    assert_eq!(
        r["Successful"],
        json!([{"Id": "a", "MessageId": "m-1", "SequenceNumber": 1},
               {"Id": "b", "MessageId": "m-1", "SequenceNumber": 1},
               {"Id": "c", "MessageId": "m-2", "SequenceNumber": 2}])
    );
    assert_eq!(r["Failed"], json!([]));
}

#[test]
fn batch_validation_errors_do_nothing() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let eleven: Vec<Value> = (0..11)
        .map(|i| json!({"Id": format!("e{i}"), "Body": "x"}))
        .collect();
    let cases = [
        (json!([]), "EmptyBatchRequest"),
        (Value::Array(eleven), "TooManyEntriesInBatchRequest"),
        (json!([{"Id": "a", "Body": "x"}, 5]), "InvalidBatchEntryId"),
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
        (json!({}), "InvalidParameterValue"),
    ];
    for (entries, code) in cases {
        assert_eq!(
            h.err(json!({"Command": "SendMessageBatch", "QueueName": "q", "Entries": entries})),
            code,
            "{entries}"
        );
    }
    assert!(receive(&mut h, "q", 10).is_empty());
    // The next accepted message still gets number 1.
    let r = send_batch(&mut h, "q", json!([{"Id": "a", "Body": "x"}]));
    assert_eq!(r["Successful"][0]["MessageId"], "m-1");
}

#[test]
fn batch_limits_accept_boundaries() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let long_id = send_batch(&mut h, "q", json!([{"Id": "x".repeat(80), "Body": "x"}]));
    assert_eq!(long_id["Successful"][0]["Id"], "x".repeat(80));
    let ten: Vec<Value> = (0..10)
        .map(|i| json!({"Id": format!("A_-{i}"), "Body": "x"}))
        .collect();
    let r = send_batch(&mut h, "q", Value::Array(ten));
    assert_eq!(r["Successful"].as_array().unwrap().len(), 10);
}

#[test]
fn check_order_for_batches() {
    let mut h = Harness::new();
    assert_eq!(
        h.err(json!({"Command": "SendMessageBatch", "Entries": []})),
        "MissingParameter"
    );
    assert_eq!(
        h.err(json!({"Command": "SendMessageBatch", "QueueName": "q"})),
        "MissingParameter"
    );
    assert_eq!(
        h.err(json!({"Command": "DeleteMessageBatch", "QueueName": "q", "Entries": []})),
        "QueueDoesNotExist"
    );
    assert_eq!(
        h.err(json!({"Command": "DeleteMessageBatch", "QueueName": 1, "Entries": []})),
        "InvalidParameterValue"
    );
}

#[test]
fn delete_batch_sees_handles_used_earlier_in_the_batch() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    send_batch(
        &mut h,
        "q",
        json!([{"Id": "a", "Body": "x"}, {"Id": "b", "Body": "y"}]),
    );
    let got = receive(&mut h, "q", 2);
    let (h1, h2) = (
        got[0]["ReceiptHandle"].clone(),
        got[1]["ReceiptHandle"].clone(),
    );
    let r = h.run(
        json!({"Command": "DeleteMessageBatch", "QueueName": "q", "Entries": [
        {"Id": "1", "ReceiptHandle": h1}, {"Id": "2", "ReceiptHandle": h1},
        {"Id": "3", "ReceiptHandle": "r-99"}, {"Id": "4"}, {"Id": "5", "ReceiptHandle": h2}]}),
    );
    assert_eq!(r["Successful"], json!([{"Id": "1"}, {"Id": "5"}]));
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
        vec![
            ("2", "ReceiptHandleExpired"),
            ("3", "ReceiptHandleInvalid"),
            ("4", "MissingParameter"),
        ]
    );
    h.tick(31);
    assert!(receive(&mut h, "q", 10).is_empty());
}

#[test]
fn message_limit_applies_within_a_batch() {
    // 100000 messages is too slow to send singly in a test; use the batch itself.
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    for round in 0..10_000 {
        let entries: Vec<Value> = (0..10)
            .map(|i| json!({"Id": format!("i{i}"), "Body": "x"}))
            .collect();
        let r = send_batch(&mut h, "q", Value::Array(entries));
        assert_eq!(r["Failed"], json!([]), "round {round}");
    }
    let r = send_batch(&mut h, "q", json!([{"Id": "a", "Body": "x"}]));
    assert_eq!(r["Successful"], json!([]));
    assert_eq!(r["Failed"][0]["error"]["code"], "LimitExceeded");
}
