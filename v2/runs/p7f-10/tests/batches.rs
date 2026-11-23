mod common;

use common::Harness;
use serde_json::{Value, json};

fn setup(fifo: bool) -> Harness {
    let mut h = Harness::new();
    let (name, attrs) = if fifo {
        ("q.fifo", json!({"Fifo": true}))
    } else {
        ("q", json!({}))
    };
    h.ok(json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attrs}));
    h
}

fn send_batch(entries: Value) -> Value {
    json!({"Command": "SendMessageBatch", "QueueName": "q", "Entries": entries})
}

fn delete_batch(entries: Value) -> Value {
    json!({"Command": "DeleteMessageBatch", "QueueName": "q", "Entries": entries})
}

#[test]
fn send_batch_in_order_with_partial_failure() {
    let mut h = setup(false);
    h.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"MaxMessageBytes": 3}}));
    let r = h.ok(send_batch(json!([
        {"Id": "a", "Body": "one"},
        {"Id": "b", "Body": "toolong"},
        {"Id": "c"},
        {"Id": "d", "Body": "x", "GroupId": "g"},
        {"Id": "e", "Body": "two", "DelaySeconds": 10, "Extra": 1},
    ])));
    assert_eq!(
        r["Successful"],
        json!([{"Id": "a", "MessageId": "m-1"}, {"Id": "e", "MessageId": "m-2"}])
    );
    let failed = r["Failed"].as_array().unwrap();
    let got: Vec<(&str, &str)> = failed
        .iter()
        .map(|f| (f["Id"].as_str().unwrap(), f["error"]["code"].as_str().unwrap()))
        .collect();
    assert_eq!(
        got,
        [
            ("b", "InvalidParameterValue"),
            ("c", "MissingParameter"),
            ("d", "UnsupportedOperation"),
        ]
    );
    assert!(failed[0]["error"]["message"].is_string());
    let c = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}))["Counts"].clone();
    assert_eq!(c, json!({"Visible": 1, "InFlight": 0, "Delayed": 1}));
}

#[test]
fn all_entries_failing_is_still_a_success() {
    let mut h = setup(false);
    let r = h.ok(send_batch(json!([{"Id": "a", "Body": ""}])));
    assert_eq!(r["Successful"], json!([]));
    assert_eq!(r["Failed"].as_array().unwrap().len(), 1);
}

#[test]
fn entries_see_earlier_entries_on_fifo() {
    let mut h = setup(true);
    let r = h.ok(json!({"Command": "SendMessageBatch", "QueueName": "q.fifo", "Entries": [
        {"Id": "a", "Body": "x", "GroupId": "g", "DeduplicationId": "d"},
        {"Id": "b", "Body": "y", "GroupId": "g", "DeduplicationId": "d"},
        {"Id": "c", "Body": "z", "GroupId": "g", "DeduplicationId": "e"},
        {"Id": "d", "Body": "z", "GroupId": "g", "DeduplicationId": "f", "DelaySeconds": 1},
        {"Id": "e", "Body": "z"},
    ]}));
    assert_eq!(
        r["Successful"],
        json!([
            {"Id": "a", "MessageId": "m-1", "SequenceNumber": 1},
            {"Id": "b", "MessageId": "m-1", "SequenceNumber": 1},
            {"Id": "c", "MessageId": "m-2", "SequenceNumber": 2},
        ])
    );
    let codes: Vec<&str> = r["Failed"].as_array().unwrap().iter().map(|f| f["error"]["code"].as_str().unwrap()).collect();
    assert_eq!(codes, ["UnsupportedOperation", "MissingParameter"]);
}

#[test]
fn send_batch_hits_message_limit_midway() {
    let mut h = setup(false);
    // Fill to the limit cheaply by sending in batches.
    for _ in 0..10_000 {
        let entries: Vec<Value> = (0..10).map(|i| json!({"Id": format!("i{i}"), "Body": "x"})).collect();
        let r = h.run(send_batch(json!(entries)));
        assert_eq!(r["Failed"], json!([]));
    }
    let r = h.ok(send_batch(json!([{"Id": "a", "Body": "x"}])));
    assert_eq!(r["Failed"][0]["error"]["code"], "LimitExceeded");
}

#[test]
fn delete_batch_results() {
    let mut h = setup(false);
    for b in ["a", "b", "c"] {
        h.ok(json!({"Command": "SendMessage", "QueueName": "q", "Body": b}));
    }
    let got = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 3}));
    let handles: Vec<Value> = got["Messages"].as_array().unwrap().iter().map(|m| m["ReceiptHandle"].clone()).collect();
    let r = h.ok(delete_batch(json!([
        {"Id": "1", "ReceiptHandle": handles[0]},
        {"Id": "2", "ReceiptHandle": "r-99"},
        {"Id": "3", "ReceiptHandle": handles[0]},
        {"Id": "4"},
        {"Id": "5", "ReceiptHandle": 5},
        {"Id": "6", "ReceiptHandle": handles[2], "Body": "ignored"},
    ])));
    assert_eq!(r["Successful"], json!([{"Id": "1"}, {"Id": "6"}]));
    let codes: Vec<(&str, &str)> = r["Failed"].as_array().unwrap().iter()
        .map(|f| (f["Id"].as_str().unwrap(), f["error"]["code"].as_str().unwrap())).collect();
    assert_eq!(
        codes,
        [
            ("2", "ReceiptHandleInvalid"),
            ("3", "ReceiptHandleExpired"),
            ("4", "MissingParameter"),
            ("5", "InvalidParameterValue"),
        ]
    );
    let c = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}))["Counts"].clone();
    assert_eq!(c, json!({"Visible": 0, "InFlight": 1, "Delayed": 0}));
}

#[test]
fn request_level_errors() {
    let mut h = setup(false);
    for build in [send_batch as fn(Value) -> Value, delete_batch] {
        assert_eq!(h.err(build(json!([]))), "EmptyBatchRequest");
        let eleven: Vec<Value> = (0..11).map(|i| json!({"Id": format!("i{i}"), "Body": "x"})).collect();
        assert_eq!(h.err(build(json!(eleven))), "TooManyEntriesInBatchRequest");
        // Too many wins over bad ids; bad ids win over duplicates.
        assert_eq!(h.err(build(json!(vec![json!(1); 11]))), "TooManyEntriesInBatchRequest");
        for bad in [json!(1), json!({}), json!({"Id": 1}), json!({"Id": null}), json!({"Id": ""}),
                    json!({"Id": "a b"}), json!({"Id": "x".repeat(81)})] {
            assert_eq!(h.err(build(json!([{"Id": "a"}, {"Id": "a"}, bad]))), "InvalidBatchEntryId");
        }
        assert_eq!(h.err(build(json!([{"Id": "a"}, {"Id": "b"}, {"Id": "a"}]))), "BatchEntryIdsNotDistinct");
        assert_eq!(h.err(build(json!({"Id": "a"}))), "InvalidParameterValue");
        h.ok(build(json!([{"Id": "x".repeat(80)}, {"Id": "A_-9"}])));
    }
    // Request-level failure does nothing.
    h.err(send_batch(json!([{"Id": "a", "Body": "x"}, {"Id": "a", "Body": "y"}])));
    assert_eq!(h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}))["Counts"]["Visible"], 0);
    // Order: required, then value type, then lookup, then batch rules.
    assert_eq!(h.err(json!({"Command": "SendMessageBatch", "QueueName": "q"})), "MissingParameter");
    assert_eq!(h.err(json!({"Command": "DeleteMessageBatch", "Entries": []})), "MissingParameter");
    assert_eq!(h.err(json!({"Command": "SendMessageBatch", "QueueName": "nope", "Entries": 1})), "InvalidParameterValue");
    assert_eq!(h.err(json!({"Command": "SendMessageBatch", "QueueName": "nope", "Entries": []})), "QueueDoesNotExist");
    assert_eq!(h.err(json!({"Command": "DeleteMessageBatch", "QueueName": "nope", "Entries": []})), "QueueDoesNotExist");
}
