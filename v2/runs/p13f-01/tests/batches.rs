mod common;

use common::Harness;
use serde_json::{Value, json};

fn fifo_harness() -> Harness {
    let mut h = Harness::new();
    h.ok(json!({"Command": "CreateQueue", "QueueName": "f.fifo", "Attributes": {"Fifo": true}}));
    h
}

fn standard_harness() -> Harness {
    let mut h = Harness::new();
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    h
}

fn send_batch(queue: &str, entries: Value) -> Value {
    json!({"Command": "SendMessageBatch", "QueueName": queue, "Entries": entries})
}

fn delete_batch(queue: &str, entries: Value) -> Value {
    json!({"Command": "DeleteMessageBatch", "QueueName": queue, "Entries": entries})
}

fn entries_with_ids(ids: impl IntoIterator<Item = String>) -> Value {
    ids.into_iter()
        .map(|id| json!({"Id": id, "Body": "b"}))
        .collect()
}

#[test]
fn send_batch_returns_ids_in_entry_order() {
    let mut h = standard_harness();
    let r = h.ok(send_batch(
        "q",
        json!([{"Id": "x", "Body": "a"}, {"Id": "y", "Body": "b"}]),
    ));
    assert_eq!(
        r["Successful"],
        json!([{"Id": "x", "MessageId": "m-1"}, {"Id": "y", "MessageId": "m-2"}])
    );
    assert_eq!(r["Failed"], json!([]));
    let r = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 10}));
    let bodies: Vec<&Value> = r["Messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| &m["Body"])
        .collect();
    assert_eq!(bodies, [&json!("a"), &json!("b")]);
}

#[test]
fn failed_entries_do_not_stop_others_or_use_message_ids() {
    let mut h = standard_harness();
    let r = h.ok(send_batch(
        "q",
        json!([
            {"Id": "a", "Body": "ok"},
            {"Id": "nobody"},
            {"Id": "bad-delay", "Body": "x", "DelaySeconds": 901},
            {"Id": "group", "Body": "x", "GroupId": "g"},
            {"Id": "last", "Body": "ok", "DelaySeconds": null},
        ]),
    ));
    assert_eq!(
        r["Successful"],
        json!([{"Id": "a", "MessageId": "m-1"}, {"Id": "last", "MessageId": "m-2"}])
    );
    let codes: Vec<(&Value, &Value)> = r["Failed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| (&f["Id"], &f["error"]["code"]))
        .collect();
    assert_eq!(
        codes,
        [
            (&json!("nobody"), &json!("MissingParameter")),
            (&json!("bad-delay"), &json!("InvalidParameterValue")),
            (&json!("group"), &json!("UnsupportedOperation")),
        ]
    );
    assert!(r["Failed"][0]["error"]["message"].is_string());
}

#[test]
fn fifo_entries_see_sequence_numbers_and_deduplication_of_earlier_entries() {
    let mut h = fifo_harness();
    let r = h.ok(send_batch(
        "f.fifo",
        json!([
            {"Id": "1", "Body": "a", "GroupId": "g", "DeduplicationId": "d"},
            {"Id": "2", "Body": "b", "GroupId": "g", "DeduplicationId": "d"},
            {"Id": "3", "Body": "c", "GroupId": "g", "DeduplicationId": "e"},
            {"Id": "4", "Body": "c", "GroupId": "g"},
            {"Id": "5", "Body": "c", "GroupId": "g", "DelaySeconds": 1},
        ]),
    ));
    assert_eq!(
        r["Successful"],
        json!([
            {"Id": "1", "MessageId": "m-1", "SequenceNumber": 1},
            {"Id": "2", "MessageId": "m-1", "SequenceNumber": 1},
            {"Id": "3", "MessageId": "m-2", "SequenceNumber": 2},
        ])
    );
    assert_eq!(r["Failed"][0]["Id"], "4");
    assert_eq!(r["Failed"][0]["error"]["code"], "MissingParameter");
    assert_eq!(r["Failed"][1]["Id"], "5");
    assert_eq!(r["Failed"][1]["error"]["code"], "UnsupportedOperation");
}

#[test]
fn entries_respect_max_message_bytes() {
    let mut h = Harness::new();
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": {"MaxMessageBytes": 2}}));
    let r = h.ok(send_batch(
        "q",
        json!([{"Id": "a", "Body": "ab"}, {"Id": "b", "Body": "abc"}]),
    ));
    assert_eq!(r["Successful"], json!([{"Id": "a", "MessageId": "m-1"}]));
    assert_eq!(r["Failed"][0]["Id"], "b");
    assert_eq!(r["Failed"][0]["error"]["code"], "InvalidParameterValue");
}

#[test]
fn entries_see_the_message_limit() {
    let mut h = standard_harness();
    for _ in 0..10_000 {
        h.ok(send_batch(
            "q",
            entries_with_ids((0..10).map(|i| i.to_string())),
        ));
    }
    let r = h.ok(send_batch("q", json!([{"Id": "a", "Body": "x"}])));
    assert_eq!(r["Failed"][0]["error"]["code"], "LimitExceeded");
    assert_eq!(r["Successful"], json!([]));
}

#[test]
fn batch_shape_errors_in_rule_order() {
    let mut h = standard_harness();
    let ten = entries_with_ids((0..10).map(|i| i.to_string()));
    assert_eq!(
        h.ok(send_batch("q", ten.clone()))["Successful"]
            .as_array()
            .unwrap()
            .len(),
        10
    );
    let eleven = entries_with_ids((0..11).map(|i| i.to_string()));
    let long_id = "i".repeat(81);
    let cases = [
        (json!([]), "EmptyBatchRequest"),
        (eleven, "TooManyEntriesInBatchRequest"),
        (json!([{"Body": "a"}]), "InvalidBatchEntryId"),
        (json!([{"Id": null, "Body": "a"}]), "InvalidBatchEntryId"),
        (json!([{"Id": 1, "Body": "a"}]), "InvalidBatchEntryId"),
        (json!([{"Id": "", "Body": "a"}]), "InvalidBatchEntryId"),
        (json!([{"Id": "a b", "Body": "a"}]), "InvalidBatchEntryId"),
        (json!([{"Id": long_id, "Body": "a"}]), "InvalidBatchEntryId"),
        (
            json!([{"Id": "a", "Body": "a"}, "text"]),
            "InvalidBatchEntryId",
        ),
        (
            json!([{"Id": "a", "Body": "a"}, {"Id": "a", "Body": "b"}]),
            "BatchEntryIdsNotDistinct",
        ),
        (json!([{"Id": "a"}, {"Id": "a"}, 5]), "InvalidBatchEntryId"),
    ];
    for (entries, code) in cases {
        assert_eq!(h.err(send_batch("q", entries.clone())), code, "{entries}");
        assert_eq!(h.err(delete_batch("q", entries.clone())), code, "{entries}");
    }
    let r = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(r["Counts"]["Visible"], 10);
}

#[test]
fn id_of_80_characters_is_accepted() {
    let mut h = standard_harness();
    let id = "A-_9".repeat(20);
    let r = h.ok(send_batch("q", json!([{"Id": id.clone(), "Body": "a"}])));
    assert_eq!(r["Successful"][0]["Id"], json!(id));
}

#[test]
fn check_order_before_entry_rules() {
    let mut h = standard_harness();
    assert_eq!(
        h.err(json!({"Command": "SendMessageBatch", "QueueName": "q"})),
        "MissingParameter"
    );
    assert_eq!(
        h.err(json!({"Command": "DeleteMessageBatch", "Entries": []})),
        "MissingParameter"
    );
    assert_eq!(h.err(send_batch("q", json!("x"))), "InvalidParameterValue");
    assert_eq!(h.err(send_batch("q", json!({}))), "InvalidParameterValue");
    assert_eq!(h.err(send_batch("q", Value::Null)), "MissingParameter");
    assert_eq!(
        h.err(send_batch(&json!(5).to_string(), json!([]))),
        "QueueDoesNotExist"
    );
    assert_eq!(
        h.err(json!({"Command": "SendMessageBatch", "QueueName": 5, "Entries": []})),
        "InvalidParameterValue"
    );
    assert_eq!(h.err(send_batch("nope", json!([]))), "QueueDoesNotExist");
    assert_eq!(
        h.err(delete_batch("nope", json!([{"Id": "a"}, {"Id": "a"}]))),
        "QueueDoesNotExist"
    );
}

#[test]
fn delete_batch_deletes_and_reports_each_entry() {
    let mut h = standard_harness();
    h.ok(send_batch(
        "q",
        json!([{"Id": "a", "Body": "1"}, {"Id": "b", "Body": "2"}]),
    ));
    let r = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 2}));
    let first = r["Messages"][0]["ReceiptHandle"].clone();
    let second = r["Messages"][1]["ReceiptHandle"].clone();
    let r = h.ok(delete_batch(
        "q",
        json!([
            {"Id": "1", "ReceiptHandle": first},
            {"Id": "2", "ReceiptHandle": first},
            {"Id": "3", "ReceiptHandle": "r-99"},
            {"Id": "4"},
            {"Id": "5", "ReceiptHandle": 7},
            {"Id": "6", "ReceiptHandle": second},
        ]),
    ));
    assert_eq!(r["Successful"], json!([{"Id": "1"}, {"Id": "6"}]));
    let failed: Vec<(&Value, &Value)> = r["Failed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| (&f["Id"], &f["error"]["code"]))
        .collect();
    assert_eq!(
        failed,
        [
            (&json!("2"), &json!("ReceiptHandleExpired")),
            (&json!("3"), &json!("ReceiptHandleInvalid")),
            (&json!("4"), &json!("MissingParameter")),
            (&json!("5"), &json!("InvalidParameterValue")),
        ]
    );
    let r = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(
        r["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn delete_batch_handle_of_another_queue_is_expired() {
    let mut h = standard_harness();
    h.ok(json!({"Command": "CreateQueue", "QueueName": "other"}));
    h.ok(json!({"Command": "SendMessage", "QueueName": "other", "Body": "x"}));
    let r = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "other"}));
    let handle = r["Messages"][0]["ReceiptHandle"].clone();
    let r = h.ok(delete_batch(
        "q",
        json!([{"Id": "a", "ReceiptHandle": handle}]),
    ));
    assert_eq!(r["Failed"][0]["error"]["code"], "ReceiptHandleExpired");
    assert_eq!(r["Successful"], json!([]));
}

#[test]
fn unknown_entry_members_are_ignored() {
    let mut h = standard_harness();
    let r = h.ok(send_batch(
        "q",
        json!([{"Id": "a", "Body": "x", "Extra": [1]}]),
    ));
    assert_eq!(r["Successful"], json!([{"Id": "a", "MessageId": "m-1"}]));
}
