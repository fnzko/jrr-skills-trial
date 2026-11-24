mod common;

use common::Harness;
use serde_json::{Value, json};

fn send_batch(h: &mut Harness, queue: &str, entries: Value) -> Value {
    h.run(json!({"Command": "SendMessageBatch", "QueueName": queue, "Entries": entries}))
}

fn delete_batch(h: &mut Harness, queue: &str, entries: Value) -> Value {
    h.run(json!({"Command": "DeleteMessageBatch", "QueueName": queue, "Entries": entries}))
}

fn code(response: &Value) -> &str {
    assert_eq!(response["ok"], false, "{response}");
    response["error"]["code"].as_str().unwrap()
}

fn queue(attributes: Value) -> Harness {
    let mut h = Harness::new();
    let name = if attributes["Fifo"] == true { "q.fifo" } else { "q" };
    h.ok(json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attributes}));
    h
}

#[test]
fn send_batch_numbers_messages_in_order() {
    let mut h = queue(json!({}));
    let r = send_batch(&mut h, "q", json!([{"Id": "a", "Body": "x"}, {"Id": "b", "Body": "y"}]));
    assert_eq!(r["ok"], true);
    assert_eq!(
        r["Successful"],
        json!([{"Id": "a", "MessageId": "m-1"}, {"Id": "b", "MessageId": "m-2"}])
    );
    assert_eq!(r["Failed"], json!([]));
}

#[test]
fn failed_entries_are_reported_and_do_not_stop_others() {
    let mut h = queue(json!({"MaxMessageBytes": 2}));
    let r = send_batch(
        &mut h,
        "q",
        json!([
            {"Id": "ok1", "Body": "ab"},
            {"Id": "long", "Body": "abc"},
            {"Id": "nobody"},
            {"Id": "typed", "Body": 5},
            {"Id": "group", "Body": "a", "GroupId": "g"},
            {"Id": "ok2", "Body": "a", "DelaySeconds": 5, "Extra": 1},
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
        .map(|f| (f["Id"].as_str().unwrap(), f["error"]["code"].as_str().unwrap()))
        .collect();
    assert_eq!(
        failed,
        [
            ("long", "InvalidParameterValue"),
            ("nobody", "MissingParameter"),
            ("typed", "InvalidParameterValue"),
            ("group", "UnsupportedOperation"),
        ]
    );
    assert!(r["Failed"][0]["error"]["message"].is_string());
}

#[test]
fn null_body_counts_as_absent() {
    let mut h = queue(json!({}));
    let r = send_batch(&mut h, "q", json!([{"Id": "a", "Body": null}]));
    assert_eq!(r["Failed"][0]["error"]["code"], "MissingParameter");
}

#[test]
fn fifo_entries_see_earlier_entries() {
    let mut h = queue(json!({"Fifo": true}));
    let r = send_batch(
        &mut h,
        "q.fifo",
        json!([
            {"Id": "a", "Body": "x", "GroupId": "g", "DeduplicationId": "d"},
            {"Id": "b", "Body": "y", "GroupId": "g", "DeduplicationId": "d"},
            {"Id": "c", "Body": "z", "GroupId": "g", "DeduplicationId": "e"},
            {"Id": "d", "Body": "z", "DeduplicationId": "f"},
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
    assert_eq!(r["Failed"][0]["error"]["code"], "MissingParameter");
}

#[test]
fn message_limit_applies_inside_a_batch() {
    let mut h = queue(json!({}));
    for _ in 0..10_000 {
        let entries: Vec<Value> = (0..10).map(|i| json!({"Id": format!("e{i}"), "Body": "x"})).collect();
        assert_eq!(send_batch(&mut h, "q", json!(entries))["Failed"], json!([]));
    }
    let r = send_batch(&mut h, "q", json!([{"Id": "a", "Body": "x"}]));
    assert_eq!(r["Failed"][0]["error"]["code"], "LimitExceeded");
    assert_eq!(r["Successful"], json!([]));
}

#[test]
fn batch_shape_rules_in_order() {
    let mut h = queue(json!({}));
    let entry = |id: Value| json!({"Id": id, "Body": "x"});
    let eleven: Vec<Value> = (0..11).map(|i| entry(json!(format!("e{i}")))).collect();
    let ten: Vec<Value> = (0..10).map(|i| entry(json!(format!("e{i}")))).collect();
    assert_eq!(code(&send_batch(&mut h, "q", json!([]))), "EmptyBatchRequest");
    assert_eq!(
        code(&send_batch(&mut h, "q", json!(eleven))),
        "TooManyEntriesInBatchRequest"
    );
    assert_eq!(send_batch(&mut h, "q", json!(ten))["Successful"].as_array().unwrap().len(), 10);
    for bad in [
        json!([5]),
        json!([{"Body": "x"}]),
        json!([{"Id": null, "Body": "x"}]),
        json!([entry(json!(7))]),
        json!([entry(json!(""))]),
        json!([entry(json!("a b"))]),
        json!([entry(json!("a".repeat(81)))]),
        json!([entry(json!("ok")), "str"]),
    ] {
        assert_eq!(code(&send_batch(&mut h, "q", bad)), "InvalidBatchEntryId");
    }
    let edge = send_batch(&mut h, "q", json!([entry(json!("a".repeat(80)))]));
    assert_eq!(edge["ok"], true);
    assert_eq!(
        code(&send_batch(&mut h, "q", json!([entry(json!("a")), entry(json!("a"))]))),
        "BatchEntryIdsNotDistinct"
    );
    // an invalid Id wins over a duplicate that comes earlier in the list
    assert_eq!(
        code(&send_batch(&mut h, "q", json!([entry(json!("a")), entry(json!("a")), entry(json!(1))]))),
        "InvalidBatchEntryId"
    );
}

#[test]
fn rejected_batch_does_nothing() {
    let mut h = queue(json!({}));
    let r = send_batch(
        &mut h,
        "q",
        json!([{"Id": "a", "Body": "x"}, {"Id": "a", "Body": "y"}]),
    );
    assert_eq!(code(&r), "BatchEntryIdsNotDistinct");
    let counts = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}))["Counts"].clone();
    assert_eq!(counts["Visible"], 0);
    let next = h.ok(json!({"Command": "SendMessage", "QueueName": "q", "Body": "z"}));
    assert_eq!(next["MessageId"], "m-1");
}

#[test]
fn check_order_before_shape_rules() {
    let mut h = queue(json!({}));
    assert_eq!(h.err(json!({"Command": "SendMessageBatch", "QueueName": "q"})), "MissingParameter");
    assert_eq!(h.err(json!({"Command": "SendMessageBatch", "Entries": []})), "MissingParameter");
    assert_eq!(
        h.err(json!({"Command": "DeleteMessageBatch", "QueueName": "q", "Entries": {}})),
        "InvalidParameterValue"
    );
    assert_eq!(
        h.err(json!({"Command": "SendMessageBatch", "QueueName": 1, "Entries": []})),
        "InvalidParameterValue"
    );
    assert_eq!(
        h.err(json!({"Command": "SendMessageBatch", "QueueName": "nope", "Entries": []})),
        "QueueDoesNotExist"
    );
}

#[test]
fn delete_batch_uses_handles_once() {
    let mut h = queue(json!({}));
    send_batch(&mut h, "q", json!([{"Id": "a", "Body": "x"}, {"Id": "b", "Body": "y"}]));
    let received = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 2}));
    let first = received["Messages"][0]["ReceiptHandle"].clone();
    let second = received["Messages"][1]["ReceiptHandle"].clone();
    let r = delete_batch(
        &mut h,
        "q",
        json!([
            {"Id": "1", "ReceiptHandle": first},
            {"Id": "2", "ReceiptHandle": first},
            {"Id": "3", "ReceiptHandle": "r-99"},
            {"Id": "4"},
            {"Id": "5", "ReceiptHandle": second},
        ]),
    );
    assert_eq!(r["ok"], true);
    assert_eq!(r["Successful"], json!([{"Id": "1"}, {"Id": "5"}]));
    let codes: Vec<&str> = r["Failed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["error"]["code"].as_str().unwrap())
        .collect();
    assert_eq!(codes, ["ReceiptHandleExpired", "ReceiptHandleInvalid", "MissingParameter"]);
    let ids: Vec<&str> = r["Failed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["Id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["2", "3", "4"]);
}

#[test]
fn delete_batch_shape_rules() {
    let mut h = queue(json!({}));
    let r = delete_batch(&mut h, "q", json!([]));
    assert_eq!(code(&r), "EmptyBatchRequest");
    let r = delete_batch(&mut h, "q", json!([{"Id": "a"}, {"Id": "a"}]));
    assert_eq!(code(&r), "BatchEntryIdsNotDistinct");
    let r = delete_batch(&mut h, "q", json!([{"ReceiptHandle": "r-1"}]));
    assert_eq!(code(&r), "InvalidBatchEntryId");
    let r = delete_batch(&mut h, "q", json!([{"Id": "a", "ReceiptHandle": 5}]));
    assert_eq!(r["Failed"][0]["error"]["code"], "InvalidParameterValue");
}
