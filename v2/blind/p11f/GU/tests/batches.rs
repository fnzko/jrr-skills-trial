mod common;

use common::Harness;
use serde_json::{Value, json};

fn setup() -> Harness {
    let mut h = Harness::new();
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    h
}

fn send_batch(h: &mut Harness, entries: Value) -> Value {
    h.ok(json!({"Command": "SendMessageBatch", "QueueName": "q", "Entries": entries}))
}

fn batch_err(h: &mut Harness, cmd: &str, entries: Value) -> String {
    h.err(json!({"Command": cmd, "QueueName": "q", "Entries": entries}))
}

#[test]
fn send_batch_results_in_order() {
    let mut h = setup();
    let r = send_batch(
        &mut h,
        json!([
            {"Id": "a", "Body": "one"},
            {"Id": "b"},
            {"Id": "c", "Body": "three", "DelaySeconds": 5, "Extra": 1},
            {"Id": "d", "Body": "x", "GroupId": "g"},
            {"Id": "e", "Body": "five", "DelaySeconds": null},
        ]),
    );
    assert_eq!(
        r["Successful"],
        json!([{"Id": "a", "MessageId": "m-1"}, {"Id": "c", "MessageId": "m-2"}, {"Id": "e", "MessageId": "m-3"}])
    );
    let failed = r["Failed"].as_array().unwrap();
    assert_eq!(failed.len(), 2);
    assert_eq!(failed[0]["Id"], "b");
    assert_eq!(failed[0]["error"]["code"], "MissingParameter");
    assert!(failed[0]["error"]["message"].is_string());
    assert_eq!(failed[1]["Id"], "d");
    assert_eq!(failed[1]["error"]["code"], "UnsupportedOperation");
    let c = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}))["Counts"].clone();
    assert_eq!(c, json!({"Visible": 2, "InFlight": 0, "Delayed": 1}));
}

#[test]
fn entry_value_errors_and_all_failed_is_success() {
    let mut h = setup();
    let r = send_batch(
        &mut h,
        json!([{"Id": "a", "Body": ""}, {"Id": "b", "Body": 5}, {"Id": "c", "Body": "x", "DelaySeconds": 901}]),
    );
    assert_eq!(r["Successful"], json!([]));
    for f in r["Failed"].as_array().unwrap() {
        assert_eq!(f["error"]["code"], "InvalidParameterValue");
    }
}

#[test]
fn batch_level_errors_and_order() {
    let mut h = setup();
    assert_eq!(h.err(json!({"Command": "SendMessageBatch", "QueueName": "q"})), "MissingParameter");
    assert_eq!(h.err(json!({"Command": "DeleteMessageBatch", "Entries": []})), "MissingParameter");
    assert_eq!(batch_err(&mut h, "SendMessageBatch", json!({})), "InvalidParameterValue");
    assert_eq!(batch_err(&mut h, "SendMessageBatch", json!([])), "EmptyBatchRequest");
    let eleven: Vec<Value> = (0..11).map(|i| json!({"Id": format!("i{i}"), "Body": "x"})).collect();
    assert_eq!(batch_err(&mut h, "SendMessageBatch", json!(eleven)), "TooManyEntriesInBatchRequest");
    // Too many beats an invalid id.
    let mut bad: Vec<Value> = (0..11).map(|_| json!(1)).collect();
    assert_eq!(batch_err(&mut h, "DeleteMessageBatch", json!(bad)), "TooManyEntriesInBatchRequest");
    bad.truncate(1);
    assert_eq!(batch_err(&mut h, "DeleteMessageBatch", json!(bad)), "InvalidBatchEntryId");
    for id in [json!(null), json!(1), json!(""), json!("a b"), json!("x".repeat(81))] {
        assert_eq!(
            batch_err(&mut h, "SendMessageBatch", json!([{"Id": id, "Body": "x"}])),
            "InvalidBatchEntryId"
        );
    }
    assert_eq!(batch_err(&mut h, "SendMessageBatch", json!([{"Body": "x"}])), "InvalidBatchEntryId");
    h.ok(json!({"Command": "SendMessageBatch", "QueueName": "q", "Entries": [{"Id": "x".repeat(80), "Body": "x"}]}));
    // Invalid id is reported before duplicates.
    assert_eq!(
        batch_err(&mut h, "SendMessageBatch", json!([{"Id": "a"}, {"Id": "a"}, {"Id": 1}])),
        "InvalidBatchEntryId"
    );
    assert_eq!(
        batch_err(&mut h, "SendMessageBatch", json!([{"Id": "a", "Body": "x"}, {"Id": "a", "Body": "y"}])),
        "BatchEntryIdsNotDistinct"
    );
    // Queue lookup precedes the numbered rules.
    assert_eq!(
        h.err(json!({"Command": "SendMessageBatch", "QueueName": "nope", "Entries": []})),
        "QueueDoesNotExist"
    );
    // Rejected batches did nothing: only the one valid 80-char send exists.
    let c = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}))["Counts"].clone();
    assert_eq!(c["Visible"], 1);
}

#[test]
fn entries_see_earlier_effects_fifo_dedup() {
    let mut h = Harness::new();
    h.ok(json!({"Command": "CreateQueue", "QueueName": "f.fifo", "Attributes": {"Fifo": true}}));
    let r = h.ok(json!({"Command": "SendMessageBatch", "QueueName": "f.fifo", "Entries": [
        {"Id": "a", "Body": "x", "GroupId": "g", "DeduplicationId": "d"},
        {"Id": "b", "Body": "y", "GroupId": "g", "DeduplicationId": "d"},
        {"Id": "c", "Body": "z", "GroupId": "g", "DeduplicationId": "e"},
        {"Id": "d", "Body": "z", "GroupId": "g", "DeduplicationId": "e", "DelaySeconds": 1},
    ]}));
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
}

#[test]
fn send_batch_hits_max_message_bytes_per_entry() {
    let mut h = Harness::new();
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": {"MaxMessageBytes": 2}}));
    let r = send_batch(&mut h, json!([{"Id": "a", "Body": "ab"}, {"Id": "b", "Body": "abc"}]));
    assert_eq!(r["Successful"].as_array().unwrap().len(), 1);
    assert_eq!(r["Failed"][0]["error"]["code"], "InvalidParameterValue");
}

#[test]
fn delete_batch() {
    let mut h = setup();
    send_batch(&mut h, json!([{"Id": "a", "Body": "1"}, {"Id": "b", "Body": "2"}]));
    let r = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 2}));
    let h1 = r["Messages"][0]["ReceiptHandle"].clone();
    let h2 = r["Messages"][1]["ReceiptHandle"].clone();
    let r = h.ok(json!({"Command": "DeleteMessageBatch", "QueueName": "q", "Entries": [
        {"Id": "x", "ReceiptHandle": h1},
        {"Id": "y", "ReceiptHandle": h1},          // already used by x
        {"Id": "z", "ReceiptHandle": "r-99"},
        {"Id": "w"},
        {"Id": "v", "ReceiptHandle": h2},
    ]}));
    assert_eq!(r["Successful"], json!([{"Id": "x"}, {"Id": "v"}]));
    let codes: Vec<(&str, &str)> = r["Failed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| (f["Id"].as_str().unwrap(), f["error"]["code"].as_str().unwrap()))
        .collect();
    assert_eq!(
        codes,
        [("y", "ReceiptHandleExpired"), ("z", "ReceiptHandleInvalid"), ("w", "MissingParameter")]
    );
    let c = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}))["Counts"].clone();
    assert_eq!(c, json!({"Visible": 0, "InFlight": 0, "Delayed": 0}));
}

#[test]
fn queue_limit_applies_within_batch() {
    let mut h = setup();
    // Fill to 100000 quickly via batches, then the next entry fails with LimitExceeded.
    for chunk in 0..10_000 {
        let entries: Vec<Value> = (0..10).map(|i| json!({"Id": format!("i{i}"), "Body": "x"})).collect();
        let r = send_batch(&mut h, json!(entries));
        assert_eq!(r["Failed"], json!([]), "chunk {chunk}");
    }
    let r = send_batch(&mut h, json!([{"Id": "a", "Body": "x"}]));
    assert_eq!(r["Failed"][0]["error"]["code"], "LimitExceeded");
}
