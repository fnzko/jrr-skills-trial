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

fn code(response: &Value) -> &str {
    assert_eq!(response["ok"], false, "{response}");
    response["error"]["code"].as_str().unwrap()
}

#[test]
fn send_batch_assigns_ids_in_order() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let r = send_batch(
        &mut h,
        "q",
        json!([
            {"Id": "a", "Body": "one"},
            {"Id": "b", "Body": "two", "DelaySeconds": 5},
        ]),
    );
    assert_eq!(
        r,
        json!({"ok": true, "Failed": [], "Successful": [
            {"Id": "a", "MessageId": "m-1"}, {"Id": "b", "MessageId": "m-2"}]})
    );
    let counts = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}))["Counts"].clone();
    assert_eq!(counts, json!({"Visible": 1, "InFlight": 0, "Delayed": 1}));
}

#[test]
fn failed_entries_are_reported_and_do_not_stop_others() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let r = send_batch(
        &mut h,
        "q",
        json!([
            {"Id": "a", "Body": "ok"},
            {"Id": "b"},
            {"Id": "c", "Body": ""},
            {"Id": "d", "Body": "x", "GroupId": "g"},
            {"Id": "e", "Body": "fine", "DelaySeconds": null, "Extra": 1},
        ]),
    );
    assert_eq!(r["ok"], true);
    assert_eq!(
        r["Successful"],
        json!([{"Id": "a", "MessageId": "m-1"}, {"Id": "e", "MessageId": "m-2"}])
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
            ("b", "MissingParameter"),
            ("c", "InvalidParameterValue"),
            ("d", "UnsupportedOperation")
        ]
    );
    assert!(r["Failed"][0]["error"]["message"].is_string());
}

#[test]
fn all_entries_failing_is_still_a_success() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({"MaxMessageBytes": 1}));
    let r = send_batch(&mut h, "q", json!([{"Id": "a", "Body": "too long"}]));
    assert_eq!(r["ok"], true);
    assert_eq!(r["Successful"], json!([]));
    assert_eq!(r["Failed"][0]["error"]["code"], "InvalidParameterValue");
}

#[test]
fn fifo_entries_see_earlier_entries() {
    let mut h = Harness::new();
    create(&mut h, "q.fifo", json!({"Fifo": true}));
    let r = send_batch(
        &mut h,
        "q.fifo",
        json!([
            {"Id": "a", "Body": "x", "GroupId": "g", "DeduplicationId": "d"},
            {"Id": "b", "Body": "y", "GroupId": "g", "DeduplicationId": "d"},
            {"Id": "c", "Body": "z", "GroupId": "g", "DeduplicationId": "e"},
            {"Id": "d", "Body": "w"},
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
fn queue_message_limit_applies_within_a_batch() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    // Fill the queue to 99999 messages with batches, then send three more.
    let entries: Vec<Value> = (0..10)
        .map(|i| json!({"Id": format!("e{i}"), "Body": "x"}))
        .collect();
    for _ in 0..9999 {
        send_batch(&mut h, "q", json!(entries));
    }
    for i in 0..9 {
        h.ok(json!({"Command": "SendMessage", "QueueName": "q", "Body": format!("{i}")}));
    }
    let r = send_batch(
        &mut h,
        "q",
        json!([
        {"Id": "a", "Body": "x"}, {"Id": "b", "Body": "x"}, {"Id": "c", "Body": "x"}]),
    );
    assert_eq!(r["Successful"].as_array().unwrap().len(), 1);
    assert_eq!(r["Failed"][0]["Id"], "b");
    assert_eq!(r["Failed"][0]["error"]["code"], "LimitExceeded");
    assert_eq!(r["Failed"][1]["error"]["code"], "LimitExceeded");
}

#[test]
fn batch_shape_rules() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let eleven: Vec<Value> = (0..11)
        .map(|i| json!({"Id": format!("e{i}"), "Body": "x"}))
        .collect();
    let ten: Vec<Value> = eleven[..10].to_vec();
    let long_id = "i".repeat(81);
    let cases = [
        (json!([]), "EmptyBatchRequest"),
        (json!(eleven), "TooManyEntriesInBatchRequest"),
        (json!([1]), "InvalidBatchEntryId"),
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
        (json!("x"), "InvalidParameterValue"),
        (json!({}), "InvalidParameterValue"),
    ];
    for (entries, expected) in cases {
        let r = send_batch(&mut h, "q", entries.clone());
        assert_eq!(code(&r), expected, "{entries}");
        let r = delete_batch(&mut h, "q", entries.clone());
        assert_eq!(code(&r), expected, "{entries}");
    }
    // Ten entries and an 80-character Id are fine.
    assert_eq!(
        send_batch(&mut h, "q", json!(ten))["Successful"]
            .as_array()
            .unwrap()
            .len(),
        10
    );
    let r = send_batch(&mut h, "q", json!([{"Id": "i".repeat(80), "Body": "x"}]));
    assert_eq!(r["Failed"], json!([]));
}

#[test]
fn shape_failure_does_nothing() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let r = send_batch(
        &mut h,
        "q",
        json!([{"Id": "a", "Body": "x"}, {"Id": "a", "Body": "y"}]),
    );
    assert_eq!(code(&r), "BatchEntryIdsNotDistinct");
    let next = h.ok(json!({"Command": "SendMessage", "QueueName": "q", "Body": "z"}));
    assert_eq!(next["MessageId"], "m-1");
}

#[test]
fn batch_error_order() {
    let mut h = Harness::new();
    let missing = h.err(json!({"Command": "SendMessageBatch", "QueueName": "nope"}));
    assert_eq!(missing, "MissingParameter");
    let missing = h.err(json!({"Command": "DeleteMessageBatch", "Entries": []}));
    assert_eq!(missing, "MissingParameter");
    let bad_type = h.err(json!({"Command": "SendMessageBatch", "QueueName": "nope", "Entries": 3}));
    assert_eq!(bad_type, "InvalidParameterValue");
    // The lookup comes before the numbered rules.
    let r = send_batch(&mut h, "nope", json!([]));
    assert_eq!(code(&r), "QueueDoesNotExist");
}

#[test]
fn delete_batch_reports_each_handle() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    create(&mut h, "other", json!({}));
    send_batch(
        &mut h,
        "q",
        json!([{"Id": "a", "Body": "1"}, {"Id": "b", "Body": "2"}]),
    );
    h.ok(json!({"Command": "SendMessage", "QueueName": "other", "Body": "3"}));
    h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 2}));
    h.ok(json!({"Command": "ReceiveMessage", "QueueName": "other"}));
    let r = delete_batch(
        &mut h,
        "q",
        json!([
            {"Id": "1", "ReceiptHandle": "r-1"},
            {"Id": "2", "ReceiptHandle": "r-1"},
            {"Id": "3", "ReceiptHandle": "r-3"},
            {"Id": "4", "ReceiptHandle": "r-99"},
            {"Id": "5"},
            {"Id": "6", "ReceiptHandle": 7},
            {"Id": "7", "ReceiptHandle": "r-2"},
        ]),
    );
    assert_eq!(r["Successful"], json!([{"Id": "1"}, {"Id": "7"}]));
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
            ("2", "ReceiptHandleExpired"),
            ("3", "ReceiptHandleExpired"),
            ("4", "ReceiptHandleInvalid"),
            ("5", "MissingParameter"),
            ("6", "InvalidParameterValue"),
        ]
    );
    let counts = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}))["Counts"].clone();
    assert_eq!(counts, json!({"Visible": 0, "InFlight": 0, "Delayed": 0}));
}

#[test]
fn queue_name_is_not_taken_from_entries() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    create(&mut h, "other", json!({}));
    let r = send_batch(
        &mut h,
        "q",
        json!([{"Id": "a", "Body": "x", "QueueName": "other"}]),
    );
    assert_eq!(r["Successful"].as_array().unwrap().len(), 1);
    let counts =
        h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "other"}))["Counts"].clone();
    assert_eq!(counts["Visible"], 0);
}
