mod common;

use common::Harness;
use serde_json::{Value, json};

fn setup(attrs: Value) -> Harness {
    let mut h = Harness::new();
    let name = if attrs["Fifo"] == true { "q.fifo" } else { "q" };
    h.ok(json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attrs}));
    h
}

fn send(h: &mut Harness, q: &str, entries: Value) -> Value {
    h.run(json!({"Command": "SendMessageBatch", "QueueName": q, "Entries": entries}))
}

#[test]
fn send_batch_in_order() {
    let mut h = setup(json!({}));
    let r = send(&mut h, "q", json!([
        {"Id": "a", "Body": "one"},
        {"Id": "b"},
        {"Id": "c", "Body": "two", "DelaySeconds": 5, "Extra": 1},
        {"Id": "d", "Body": "x", "GroupId": "g"},
    ]));
    assert_eq!(r["ok"], true);
    assert_eq!(r["Successful"], json!([{"Id": "a", "MessageId": "m-1"}, {"Id": "c", "MessageId": "m-2"}]));
    let failed = r["Failed"].as_array().unwrap();
    assert_eq!(failed.len(), 2);
    assert_eq!((failed[0]["Id"].clone(), failed[0]["error"]["code"].clone()), (json!("b"), json!("MissingParameter")));
    assert!(failed[0]["error"]["message"].is_string());
    assert_eq!(failed[1]["error"]["code"], "UnsupportedOperation");
    let c = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}))["Counts"].clone();
    assert_eq!(c, json!({"Visible": 1, "InFlight": 0, "Delayed": 1}));
}

#[test]
fn all_failed_is_still_success() {
    let mut h = setup(json!({}));
    let r = send(&mut h, "q", json!([{"Id": "a", "Body": 5}]));
    assert_eq!(r["ok"], true);
    assert_eq!(r["Successful"], json!([]));
    assert_eq!(r["Failed"][0]["error"]["code"], "InvalidParameterValue");
}

#[test]
fn entries_see_earlier_effects() {
    let mut h = setup(json!({"Fifo": true, "ContentBasedDeduplication": true}));
    let r = send(&mut h, "q.fifo", json!([
        {"Id": "a", "Body": "x", "GroupId": "g"},
        {"Id": "b", "Body": "x", "GroupId": "g"},
        {"Id": "c", "Body": "y", "GroupId": "g"},
    ]));
    assert_eq!(r["Successful"], json!([
        {"Id": "a", "MessageId": "m-1", "SequenceNumber": 1},
        {"Id": "b", "MessageId": "m-1", "SequenceNumber": 1},
        {"Id": "c", "MessageId": "m-2", "SequenceNumber": 2},
    ]));
}

#[test]
fn max_message_bytes_applies_to_entries() {
    let mut h = setup(json!({"MaxMessageBytes": 2}));
    let r = send(&mut h, "q", json!([{"Id": "a", "Body": "abc"}, {"Id": "b", "Body": "ab"}]));
    assert_eq!(r["Failed"][0]["error"]["code"], "InvalidParameterValue");
    assert_eq!(r["Successful"], json!([{"Id": "b", "MessageId": "m-1"}]));
}

#[test]
fn batch_level_errors() {
    let mut h = setup(json!({}));
    let ids = |n: usize| -> Value { (0..n).map(|i| json!({"Id": format!("i{i}"), "Body": "b"})).collect() };
    let code = |h: &mut Harness, e: Value| h.err(json!({"Command": "SendMessageBatch", "QueueName": "q", "Entries": e}));
    assert_eq!(code(&mut h, json!([])), "EmptyBatchRequest");
    assert_eq!(code(&mut h, ids(11)), "TooManyEntriesInBatchRequest");
    for bad in [json!(["x"]), json!([{"Body": "b"}]), json!([{"Id": 1}]), json!([{"Id": ""}]),
                json!([{"Id": "a".repeat(81)}]), json!([{"Id": "a b"}]), json!([{"Id": null}])] {
        assert_eq!(code(&mut h, bad), "InvalidBatchEntryId");
    }
    assert_eq!(code(&mut h, json!([{"Id": "a", "Body": "b"}, {"Id": "a", "Body": "b"}])), "BatchEntryIdsNotDistinct");
    // invalid Id (rule 3) beats duplicates (rule 4); nothing was sent by any failure above
    assert_eq!(code(&mut h, json!([{"Id": "a"}, {"Id": "a"}, 5])), "InvalidBatchEntryId");
    let c = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}))["Counts"].clone();
    assert_eq!(c, json!({"Visible": 0, "InFlight": 0, "Delayed": 0}));
    // limits are fine
    let r = send(&mut h, "q", ids(10));
    assert_eq!(r["Successful"].as_array().unwrap().len(), 10);
    assert_eq!(send(&mut h, "q", json!([{"Id": "a".repeat(80), "Body": "b"}, {"Id": "A-_9", "Body": "b"}]))["Failed"], json!([]));
}

#[test]
fn check_order() {
    let mut h = setup(json!({}));
    let run = |h: &mut Harness, c: Value| h.err(c);
    assert_eq!(run(&mut h, json!({"Command": "SendMessageBatch", "Entries": []})), "MissingParameter");
    assert_eq!(run(&mut h, json!({"Command": "SendMessageBatch", "QueueName": "q"})), "MissingParameter");
    assert_eq!(run(&mut h, json!({"Command": "SendMessageBatch", "QueueName": "q", "Entries": {}})), "InvalidParameterValue");
    assert_eq!(run(&mut h, json!({"Command": "DeleteMessageBatch", "QueueName": "q", "Entries": "x"})), "InvalidParameterValue");
    assert_eq!(run(&mut h, json!({"Command": "SendMessageBatch", "QueueName": "nope", "Entries": []})), "QueueDoesNotExist");
}

#[test]
fn delete_batch() {
    let mut h = setup(json!({}));
    send(&mut h, "q", json!([{"Id": "a", "Body": "x"}, {"Id": "b", "Body": "y"}]));
    let r = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 2}));
    let h1 = r["Messages"][0]["ReceiptHandle"].clone();
    let h2 = r["Messages"][1]["ReceiptHandle"].clone();
    let r = h.ok(json!({"Command": "DeleteMessageBatch", "QueueName": "q", "Entries": [
        {"Id": "1", "ReceiptHandle": h1},
        {"Id": "2", "ReceiptHandle": "r-99"},
        {"Id": "3", "ReceiptHandle": h1},   // already used by entry 1
        {"Id": "4"},
        {"Id": "5", "ReceiptHandle": h2},
    ]}));
    assert_eq!(r["Successful"], json!([{"Id": "1"}, {"Id": "5"}]));
    let codes: Vec<&str> = r["Failed"].as_array().unwrap().iter().map(|f| f["error"]["code"].as_str().unwrap()).collect();
    assert_eq!(codes, ["ReceiptHandleInvalid", "ReceiptHandleExpired", "MissingParameter"]);
    let ids: Vec<&str> = r["Failed"].as_array().unwrap().iter().map(|f| f["Id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["2", "3", "4"]);
    let c = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}))["Counts"].clone();
    assert_eq!(c, json!({"Visible": 0, "InFlight": 0, "Delayed": 0}));
}

#[test]
fn message_limit_seen_by_later_entries() {
    // entries share the message-id counter with single sends
    let mut h = setup(json!({}));
    h.ok(json!({"Command": "SendMessage", "QueueName": "q", "Body": "x"}));
    let r = send(&mut h, "q", json!([{"Id": "a", "Body": "y"}]));
    assert_eq!(r["Successful"][0]["MessageId"], "m-2");
}
