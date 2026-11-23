mod common;

use common::Harness;
use serde_json::{Value, json};

fn create(h: &mut Harness, name: &str, attributes: Value) {
    h.ok(json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attributes}));
}

fn send_batch(h: &mut Harness, queue: &str, entries: Value) -> Value {
    h.ok(json!({"Command": "SendMessageBatch", "QueueName": queue, "Entries": entries}))
}

fn delete_batch(h: &mut Harness, queue: &str, entries: Value) -> Value {
    h.ok(json!({"Command": "DeleteMessageBatch", "QueueName": queue, "Entries": entries}))
}

fn failure_code(h: &mut Harness, command: &str, entries: Value) -> String {
    h.err(json!({"Command": command, "QueueName": "q", "Entries": entries}))
}

#[test]
fn send_batch_reports_each_entry_in_order() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let r = send_batch(&mut h, "q", json!([
        {"Id": "a", "Body": "one"},
        {"Id": "b", "Body": ""},
        {"Id": "c", "Body": "three", "DelaySeconds": 5},
        {"Id": "d"},
        {"Id": "e", "Body": "five", "GroupId": "g"},
    ]));
    assert_eq!(
        r,
        json!({
            "ok": true,
            "Successful": [
                {"Id": "a", "MessageId": "m-1"},
                {"Id": "c", "MessageId": "m-2"},
            ],
            "Failed": [
                {"Id": "b", "error": {"code": "InvalidParameterValue", "message": r["Failed"][0]["error"]["message"]}},
                {"Id": "d", "error": {"code": "MissingParameter", "message": r["Failed"][1]["error"]["message"]}},
                {"Id": "e", "error": {"code": "UnsupportedOperation", "message": r["Failed"][2]["error"]["message"]}},
            ],
        })
    );
    assert!(r["Failed"][0]["error"]["message"].is_string());
    let counts = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}))["Counts"].clone();
    assert_eq!(counts, json!({"Visible": 1, "InFlight": 0, "Delayed": 1}));
}

#[test]
fn send_batch_entries_see_earlier_entries() {
    let mut h = Harness::new();
    create(&mut h, "q.fifo", json!({"Fifo": true}));
    let r = send_batch(&mut h, "q.fifo", json!([
        {"Id": "1", "Body": "x", "GroupId": "g", "DeduplicationId": "d"},
        {"Id": "2", "Body": "y", "GroupId": "g", "DeduplicationId": "d"},
        {"Id": "3", "Body": "z", "GroupId": "g", "DeduplicationId": "e"},
    ]));
    assert_eq!(
        r,
        json!({
            "ok": true,
            "Successful": [
                {"Id": "1", "MessageId": "m-1", "SequenceNumber": 1},
                {"Id": "2", "MessageId": "m-1", "SequenceNumber": 1},
                {"Id": "3", "MessageId": "m-2", "SequenceNumber": 2},
            ],
            "Failed": [],
        })
    );
}

#[test]
fn send_batch_respects_message_limit_and_max_bytes() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({"MaxMessageBytes": 3}));
    let r = send_batch(&mut h, "q", json!([
        {"Id": "a", "Body": "abc"},
        {"Id": "b", "Body": "abcd"},
    ]));
    assert_eq!(r["Successful"], json!([{"Id": "a", "MessageId": "m-1"}]));
    assert_eq!(r["Failed"][0]["Id"], "b");
    assert_eq!(r["Failed"][0]["error"]["code"], "InvalidParameterValue");
}

#[test]
fn send_batch_fills_a_queue_to_its_limit() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let entries: Vec<Value> = (0..10).map(|i| json!({"Id": format!("e{i}"), "Body": "x"})).collect();
    for _ in 0..10_000 {
        let r = send_batch(&mut h, "q", Value::Array(entries.clone()));
        assert_eq!(r["Failed"], json!([]));
    }
    let r = send_batch(&mut h, "q", json!([{"Id": "a", "Body": "x"}]));
    assert_eq!(r["Successful"], json!([]));
    assert_eq!(r["Failed"][0]["error"]["code"], "LimitExceeded");
}

#[test]
fn delete_batch_uses_handles_in_order() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    create(&mut h, "other", json!({}));
    for body in ["a", "b"] {
        h.ok(json!({"Command": "SendMessage", "QueueName": "q", "Body": body}));
    }
    h.ok(json!({"Command": "SendMessage", "QueueName": "other", "Body": "o"}));
    h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 2}));
    h.ok(json!({"Command": "ReceiveMessage", "QueueName": "other"}));
    let r = delete_batch(&mut h, "q", json!([
        {"Id": "x", "ReceiptHandle": "r-1"},
        {"Id": "y", "ReceiptHandle": "r-1"},
        {"Id": "z", "ReceiptHandle": "r-99"},
        {"Id": "w", "ReceiptHandle": "r-3"},
        {"Id": "v"},
        {"Id": "u", "ReceiptHandle": 5},
        {"Id": "t", "ReceiptHandle": "r-2"},
    ]));
    let codes: Vec<(&str, &str)> = r["Failed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| (f["Id"].as_str().unwrap(), f["error"]["code"].as_str().unwrap()))
        .collect();
    assert_eq!(
        codes,
        vec![
            ("y", "ReceiptHandleExpired"),
            ("z", "ReceiptHandleInvalid"),
            ("w", "ReceiptHandleExpired"),
            ("v", "MissingParameter"),
            ("u", "InvalidParameterValue"),
        ]
    );
    assert_eq!(r["Successful"], json!([{"Id": "x"}, {"Id": "t"}]));
    let counts = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}))["Counts"].clone();
    assert_eq!(counts, json!({"Visible": 0, "InFlight": 0, "Delayed": 0}));
}

#[test]
fn batch_shape_rules_in_order() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    for command in ["SendMessageBatch", "DeleteMessageBatch"] {
        let eleven: Vec<Value> = (0..11).map(|i| json!({"Id": format!("e{i}")})).collect();
        assert_eq!(failure_code(&mut h, command, json!([])), "EmptyBatchRequest");
        // too many beats invalid ids
        let mut bad_eleven = eleven.clone();
        bad_eleven[0] = json!(5);
        assert_eq!(failure_code(&mut h, command, Value::Array(bad_eleven)), "TooManyEntriesInBatchRequest");
        assert_eq!(failure_code(&mut h, command, Value::Array(eleven)), "TooManyEntriesInBatchRequest");
        // invalid id beats duplicate ids
        for bad in [
            json!([5]), json!([[]]), json!([{}]), json!([{"Id": null}]), json!([{"Id": 1}]),
            json!([{"Id": ""}]), json!([{"Id": "a b"}]), json!([{"Id": "x".repeat(81)}]),
            json!([{"Id": "a"}, {"Id": "a"}, {"Id": "!"}]),
        ] {
            assert_eq!(failure_code(&mut h, command, bad.clone()), "InvalidBatchEntryId", "{bad}");
        }
        assert_eq!(
            failure_code(&mut h, command, json!([{"Id": "a"}, {"Id": "b"}, {"Id": "a"}])),
            "BatchEntryIdsNotDistinct"
        );
    }
}

#[test]
fn batch_ids_may_be_80_characters_and_ten_entries_are_allowed() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let mut entries = vec![json!({"Id": "x".repeat(80), "Body": "b"})];
    entries.extend((1..10).map(|i| json!({"Id": format!("A_-{i}"), "Body": "b"})));
    let r = send_batch(&mut h, "q", Value::Array(entries));
    assert_eq!(r["Successful"].as_array().unwrap().len(), 10);
}

#[test]
fn shape_failures_do_nothing_and_checks_come_in_order() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    assert_eq!(
        failure_code(&mut h, "SendMessageBatch", json!([{"Id": "a", "Body": "x"}, {"Id": "a", "Body": "x"}])),
        "BatchEntryIdsNotDistinct"
    );
    let counts = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}))["Counts"].clone();
    assert_eq!(counts, json!({"Visible": 0, "InFlight": 0, "Delayed": 0}));
    for command in ["SendMessageBatch", "DeleteMessageBatch"] {
        assert_eq!(h.err(json!({"Command": command, "Entries": []})), "MissingParameter");
        assert_eq!(h.err(json!({"Command": command, "QueueName": "q"})), "MissingParameter");
        assert_eq!(h.err(json!({"Command": command, "QueueName": "q", "Entries": {}})), "InvalidParameterValue");
        assert_eq!(h.err(json!({"Command": command, "QueueName": 1, "Entries": []})), "InvalidParameterValue");
        // lookup comes before the batch rules
        assert_eq!(h.err(json!({"Command": command, "QueueName": "nope", "Entries": []})), "QueueDoesNotExist");
    }
}

#[test]
fn unknown_entry_members_and_null_are_ignored() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let r = send_batch(&mut h, "q", json!([
        {"Id": "a", "Body": "x", "Extra": 1, "DelaySeconds": null, "QueueName": "elsewhere"},
    ]));
    assert_eq!(r["Successful"], json!([{"Id": "a", "MessageId": "m-1"}]));
}
