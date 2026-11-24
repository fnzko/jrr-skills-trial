mod common;

use common::Harness;
use serde_json::{Value, json};

fn setup(attributes: Value) -> Harness {
    let mut h = Harness::new();
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": attributes}));
    h
}

fn send_batch(entries: Value) -> Value {
    json!({"Command": "SendMessageBatch", "QueueName": "q", "Entries": entries})
}

fn delete_batch(entries: Value) -> Value {
    json!({"Command": "DeleteMessageBatch", "QueueName": "q", "Entries": entries})
}

#[test]
fn send_batch_reports_each_entry_in_order() {
    let mut h = setup(json!({"MaxMessageBytes": 3}));
    let r = h.ok(send_batch(json!([
        {"Id": "a", "Body": "one"},
        {"Id": "b", "Body": "toolong"},
        {"Id": "c"},
        {"Id": "d", "Body": "two", "DelaySeconds": 5, "Extra": 1},
        {"Id": "e", "Body": "x", "GroupId": null},
    ])));
    assert_eq!(
        r["Successful"],
        json!([
            {"Id": "a", "MessageId": "m-1"},
            {"Id": "d", "MessageId": "m-2"},
            {"Id": "e", "MessageId": "m-3"},
        ])
    );
    let failed = r["Failed"].as_array().unwrap();
    assert_eq!(failed.len(), 2);
    assert_eq!(failed[0]["Id"], "b");
    assert_eq!(failed[0]["error"]["code"], "InvalidParameterValue");
    assert!(failed[0]["error"]["message"].is_string());
    assert_eq!(failed[1]["Id"], "c");
    assert_eq!(failed[1]["error"]["code"], "MissingParameter");
    let counts = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}))["Counts"].clone();
    assert_eq!(counts, json!({"Visible": 2, "InFlight": 0, "Delayed": 1}));
}

#[test]
fn fifo_batch_sees_earlier_entries_for_sequence_and_deduplication() {
    let mut h = Harness::new();
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q.fifo", "Attributes": {"Fifo": true}}));
    let r = h.ok(
        json!({"Command": "SendMessageBatch", "QueueName": "q.fifo", "Entries": [
            {"Id": "1", "Body": "a", "GroupId": "g", "DeduplicationId": "d"},
            {"Id": "2", "Body": "b", "GroupId": "g", "DeduplicationId": "d"},
            {"Id": "3", "Body": "c", "GroupId": "g", "DeduplicationId": "e"},
            {"Id": "4", "Body": "c", "DelaySeconds": 1, "GroupId": "g", "DeduplicationId": "f"},
        ]}),
    );
    assert_eq!(
        r["Successful"],
        json!([
            {"Id": "1", "MessageId": "m-1", "SequenceNumber": 1},
            {"Id": "2", "MessageId": "m-1", "SequenceNumber": 1},
            {"Id": "3", "MessageId": "m-2", "SequenceNumber": 2},
        ])
    );
    assert_eq!(r["Failed"][0]["Id"], "4");
    assert_eq!(r["Failed"][0]["error"]["code"], "UnsupportedOperation");
}

#[test]
fn batch_rules_and_their_order() {
    let mut h = setup(json!({}));
    let big: Vec<Value> = (0..11)
        .map(|i| json!({"Id": format!("e{i}"), "Body": "x"}))
        .collect();
    assert_eq!(h.err(send_batch(json!([]))), "EmptyBatchRequest");
    assert_eq!(h.err(delete_batch(json!([]))), "EmptyBatchRequest");
    assert_eq!(
        h.err(send_batch(json!(big))),
        "TooManyEntriesInBatchRequest"
    );
    let bad_ids = [
        json!(5),
        json!({"Body": "x"}),
        json!({"Id": null, "Body": "x"}),
        json!({"Id": 1, "Body": "x"}),
        json!({"Id": "", "Body": "x"}),
        json!({"Id": "a b", "Body": "x"}),
        json!({"Id": "a".repeat(81), "Body": "x"}),
    ];
    for bad in bad_ids {
        assert_eq!(
            h.err(send_batch(json!([bad.clone()]))),
            "InvalidBatchEntryId",
            "{bad}"
        );
    }
    h.ok(send_batch(json!([{"Id": "a".repeat(80), "Body": "x"}])));
    assert_eq!(
        h.err(send_batch(
            json!([{"Id": "a", "Body": "x"}, {"Id": "a", "Body": "y"}])
        )),
        "BatchEntryIdsNotDistinct"
    );
    assert_eq!(
        h.err(send_batch(json!([{"Id": "a"}, {"Id": "a"}, 5]))),
        "InvalidBatchEntryId"
    );
    assert_eq!(
        h.err(send_batch(json!(
            big.iter().chain(&big).collect::<Vec<_>>()
        ))),
        "TooManyEntriesInBatchRequest"
    );
}

#[test]
fn batch_request_level_checks_come_first_and_do_nothing() {
    let mut h = setup(json!({}));
    assert_eq!(
        h.err(json!({"Command": "SendMessageBatch", "QueueName": "q"})),
        "MissingParameter"
    );
    assert_eq!(
        h.err(json!({"Command": "DeleteMessageBatch", "Entries": []})),
        "MissingParameter"
    );
    assert_eq!(
        h.err(json!({"Command": "SendMessageBatch", "QueueName": "q", "Entries": {}})),
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
    h.err(send_batch(
        json!([{"Id": "a", "Body": "x"}, {"Id": "a", "Body": "x"}]),
    ));
    let counts = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}))["Counts"].clone();
    assert_eq!(counts["Visible"], 0);
    assert_eq!(
        h.ok(json!({"Command": "SendMessage", "QueueName": "q", "Body": "x"}))["MessageId"],
        "m-1"
    );
}

#[test]
fn delete_batch_handles_each_handle_once() {
    let mut h = setup(json!({}));
    for _ in 0..3 {
        h.ok(json!({"Command": "SendMessage", "QueueName": "q", "Body": "x"}));
    }
    h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 3}));
    let r = h.ok(delete_batch(json!([
        {"Id": "a", "ReceiptHandle": "r-1"},
        {"Id": "b", "ReceiptHandle": "r-1"},
        {"Id": "c", "ReceiptHandle": "r-99"},
        {"Id": "d"},
        {"Id": "e", "ReceiptHandle": "r-3"},
    ])));
    assert_eq!(r["Successful"], json!([{"Id": "a"}, {"Id": "e"}]));
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
        [
            ("b", "ReceiptHandleExpired"),
            ("c", "ReceiptHandleInvalid"),
            ("d", "MissingParameter"),
        ]
    );
    let counts = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}))["Counts"].clone();
    assert_eq!(counts, json!({"Visible": 0, "InFlight": 1, "Delayed": 0}));
}
