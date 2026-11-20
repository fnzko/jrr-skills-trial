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

fn queue(name: &str, attributes: Value) -> Harness {
    let mut h = Harness::new();
    h.ok(json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attributes}));
    h
}

#[test]
fn send_batch_numbers_messages_in_entry_order() {
    let mut h = queue("q", json!({}));
    let r = send_batch(&mut h, "q", json!([{"Id": "a", "Body": "x"}, {"Id": "b", "Body": "y", "DelaySeconds": 5}]));
    assert_eq!(
        r,
        json!({"ok": true, "Failed": [], "Successful": [
            {"Id": "a", "MessageId": "m-1"}, {"Id": "b", "MessageId": "m-2"}]})
    );
    let c = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(c["Counts"], json!({"Visible": 1, "InFlight": 0, "Delayed": 1}));
}

#[test]
fn failed_entries_do_not_stop_others_or_use_ids() {
    let mut h = queue("q", json!({"MaxMessageBytes": 3}));
    let r = send_batch(
        &mut h,
        "q",
        json!([
            {"Id": "ok1", "Body": "abc"},
            {"Id": "long", "Body": "abcd"},
            {"Id": "nobody"},
            {"Id": "typed", "Body": 5},
            {"Id": "fifoonly", "Body": "a", "GroupId": "g"},
            {"Id": "ok2", "Body": "d", "Extra": [1]},
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
            assert!(f["error"]["message"].is_string());
            (f["Id"].as_str().unwrap(), f["error"]["code"].as_str().unwrap())
        })
        .collect();
    assert_eq!(
        failed,
        [
            ("long", "InvalidParameterValue"),
            ("nobody", "MissingParameter"),
            ("typed", "InvalidParameterValue"),
            ("fifoonly", "UnsupportedOperation"),
        ]
    );
}

#[test]
fn entry_with_queue_name_member_uses_the_batch_queue() {
    let mut h = queue("q", json!({}));
    let r = send_batch(&mut h, "q", json!([{"Id": "a", "Body": "x", "QueueName": "other"}, {"Id": "b", "Body": null}]));
    assert_eq!(r["Successful"], json!([{"Id": "a", "MessageId": "m-1"}]));
    assert_eq!(r["Failed"][0]["error"]["code"], "MissingParameter");
}

#[test]
fn fifo_batch_sees_earlier_entries() {
    let mut h = queue("q.fifo", json!({"Fifo": true}));
    let r = send_batch(
        &mut h,
        "q.fifo",
        json!([
            {"Id": "a", "Body": "x", "GroupId": "g", "DeduplicationId": "d"},
            {"Id": "b", "Body": "y", "GroupId": "g", "DeduplicationId": "d"},
            {"Id": "c", "Body": "z", "GroupId": "g", "DeduplicationId": "e"},
            {"Id": "d", "Body": "z", "GroupId": "g"},
            {"Id": "e", "Body": "z", "GroupId": "g", "DelaySeconds": 1},
        ]),
    );
    assert_eq!(
        r["Successful"],
        json!([
            {"Id": "a", "MessageId": "m-1", "SequenceNumber": 1},
            // A duplicate succeeds with the original's identifiers.
            {"Id": "b", "MessageId": "m-1", "SequenceNumber": 1},
            {"Id": "c", "MessageId": "m-2", "SequenceNumber": 2},
        ])
    );
    assert_eq!(r["Failed"][0]["error"]["code"], "MissingParameter");
    assert_eq!(r["Failed"][1]["error"]["code"], "UnsupportedOperation");
}

#[test]
fn message_limit_applies_within_a_batch() {
    let mut h = queue("q", json!({}));
    for _ in 0..10_000 {
        let entries: Vec<Value> = (0..10).map(|i| json!({"Id": format!("i{i}"), "Body": "x"})).collect();
        assert_eq!(send_batch(&mut h, "q", Value::Array(entries))["Failed"], json!([]));
    }
    let r = send_batch(&mut h, "q", json!([{"Id": "a", "Body": "x"}]));
    assert_eq!(r["Failed"][0]["error"]["code"], "LimitExceeded");
}

#[test]
fn batch_rules_and_their_order() {
    let mut h = queue("q", json!({}));
    assert_eq!(code(&send_batch(&mut h, "q", json!([]))), "EmptyBatchRequest");
    let eleven: Vec<Value> = (0..11).map(|i| json!({"Id": format!("i{i}"), "Body": "x"})).collect();
    assert_eq!(code(&send_batch(&mut h, "q", Value::Array(eleven))), "TooManyEntriesInBatchRequest");
    let ten: Vec<Value> = (0..10).map(|i| json!({"Id": format!("i{i}"), "Body": "x"})).collect();
    assert_eq!(send_batch(&mut h, "q", Value::Array(ten))["ok"], true);

    let long_id = "a".repeat(81);
    for bad in [json!(["x"]), json!([{"Body": "x"}]), json!([{"Id": 1}]), json!([{"Id": null}]),
        json!([{"Id": ""}]), json!([{"Id": "a b"}]), json!([{"Id": long_id}])] {
        assert_eq!(code(&send_batch(&mut h, "q", bad.clone())), "InvalidBatchEntryId", "{bad}");
    }
    assert_eq!(send_batch(&mut h, "q", json!([{"Id": "a".repeat(80), "Body": "x"}]))["ok"], true);

    // An invalid Id (rule 3) beats a repeated one (rule 4) and too many (rule 2) beats both.
    assert_eq!(
        code(&send_batch(&mut h, "q", json!([{"Id": "a"}, {"Id": "a"}, {"Id": "b c"}]))),
        "InvalidBatchEntryId"
    );
    assert_eq!(
        code(&send_batch(&mut h, "q", json!([{"Id": "a", "Body": "x"}, {"Id": "a", "Body": "y"}]))),
        "BatchEntryIdsNotDistinct"
    );
    // Ids are case-sensitive.
    assert_eq!(send_batch(&mut h, "q", json!([{"Id": "a", "Body": "x"}, {"Id": "A", "Body": "y"}]))["ok"], true);
}

#[test]
fn rejected_batch_does_nothing() {
    let mut h = queue("q", json!({}));
    send_batch(&mut h, "q", json!([{"Id": "a", "Body": "x"}, {"Id": "a", "Body": "y"}]));
    let r = send_batch(&mut h, "q", json!([{"Id": "a", "Body": "x"}]));
    assert_eq!(r["Successful"], json!([{"Id": "a", "MessageId": "m-1"}]));
}

#[test]
fn batch_parameter_checks_come_before_the_queue_lookup() {
    let mut h = Harness::new();
    let c = |extra: Value| {
        let mut command = json!({"Command": "SendMessageBatch"});
        command.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        command
    };
    assert_eq!(h.err(c(json!({"Entries": []}))), "MissingParameter");
    assert_eq!(h.err(c(json!({"QueueName": "q"}))), "MissingParameter");
    assert_eq!(h.err(c(json!({"QueueName": "q", "Entries": {}}))), "InvalidParameterValue");
    assert_eq!(h.err(c(json!({"QueueName": 1, "Entries": []}))), "InvalidParameterValue");
    // The queue lookup precedes the numbered batch rules.
    assert_eq!(h.err(c(json!({"QueueName": "q", "Entries": []}))), "QueueDoesNotExist");
    assert_eq!(
        h.err(json!({"Command": "DeleteMessageBatch", "QueueName": "q", "Entries": []})),
        "QueueDoesNotExist"
    );
}

#[test]
fn delete_batch_handles_each_entry_in_order() {
    let mut h = queue("q", json!({}));
    send_batch(&mut h, "q", json!([{"Id": "a", "Body": "x"}, {"Id": "b", "Body": "y"}]));
    h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 2}));
    let r = delete_batch(
        &mut h,
        "q",
        json!([
            {"Id": "1", "ReceiptHandle": "r-1"},
            {"Id": "2", "ReceiptHandle": "r-1"},
            {"Id": "3", "ReceiptHandle": "r-9"},
            {"Id": "4"},
            {"Id": "5", "ReceiptHandle": 7},
            {"Id": "6", "ReceiptHandle": "r-2", "Junk": true},
        ]),
    );
    assert_eq!(r["ok"], true);
    assert_eq!(r["Successful"], json!([{"Id": "1"}, {"Id": "6"}]));
    let failed: Vec<(&str, &str)> = r["Failed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| (f["Id"].as_str().unwrap(), f["error"]["code"].as_str().unwrap()))
        .collect();
    assert_eq!(
        failed,
        [
            ("2", "ReceiptHandleExpired"),
            ("3", "ReceiptHandleInvalid"),
            ("4", "MissingParameter"),
            ("5", "InvalidParameterValue"),
        ]
    );
    let c = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(c["Counts"], json!({"Visible": 0, "InFlight": 0, "Delayed": 0}));
}

#[test]
fn delete_batch_with_only_failures_is_a_success() {
    let mut h = queue("q", json!({}));
    let r = delete_batch(&mut h, "q", json!([{"Id": "a", "ReceiptHandle": "r-1"}]));
    assert_eq!(r["ok"], true);
    assert_eq!(r["Successful"], json!([]));
    assert_eq!(r["Failed"][0]["error"]["code"], "ReceiptHandleInvalid");
}
