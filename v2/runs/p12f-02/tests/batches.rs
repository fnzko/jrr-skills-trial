mod common;

use common::Harness;
use serde_json::{Value, json};

fn create(h: &mut Harness, name: &str, attributes: Value) {
    h.ok(json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attributes}));
}

fn send_batch(queue: &str, entries: Value) -> Value {
    json!({"Command": "SendMessageBatch", "QueueName": queue, "Entries": entries})
}

fn delete_batch(queue: &str, entries: Value) -> Value {
    json!({"Command": "DeleteMessageBatch", "QueueName": queue, "Entries": entries})
}

fn failed_codes(response: &Value) -> Vec<(&str, &str)> {
    response["Failed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            (
                f["Id"].as_str().unwrap(),
                f["error"]["code"].as_str().unwrap(),
            )
        })
        .collect()
}

#[test]
fn send_batch_accepts_entries_in_order_with_consecutive_ids() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let r = h.ok(send_batch(
        "q",
        json!([{"Id": "a", "Body": "x"}, {"Id": "b", "Body": "y", "DelaySeconds": 5}]),
    ));
    assert_eq!(
        r,
        json!({"ok": true, "Failed": [], "Successful": [
            {"Id": "a", "MessageId": "m-1"}, {"Id": "b", "MessageId": "m-2"}]})
    );
    let c = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(
        c["Counts"],
        json!({"Visible": 1, "InFlight": 0, "Delayed": 1})
    );
}

#[test]
fn send_batch_entries_fail_alone_and_see_earlier_entries() {
    let mut h = Harness::new();
    create(&mut h, "f.fifo", json!({"Fifo": true}));
    let r = h.ok(send_batch(
        "f.fifo",
        json!([
            {"Id": "1", "Body": "x", "GroupId": "g", "DeduplicationId": "d"},
            {"Id": "2", "Body": "y"},
            {"Id": "3", "Body": "z", "GroupId": "g", "DeduplicationId": "d"},
            {"Id": "4", "GroupId": "g", "DeduplicationId": "e"},
            {"Id": "5", "Body": "w", "GroupId": "g", "DeduplicationId": "e", "DelaySeconds": 1},
            {"Id": "6", "Body": 7, "GroupId": "g", "DeduplicationId": "f"},
            {"Id": "7", "Body": "v", "GroupId": "g", "DeduplicationId": "h", "Extra": true},
        ]),
    ));
    assert_eq!(
        failed_codes(&r),
        vec![
            ("2", "MissingParameter"),
            ("4", "MissingParameter"),
            ("5", "UnsupportedOperation"),
            ("6", "InvalidParameterValue")
        ]
    );
    assert_eq!(
        r["Successful"],
        json!([
            {"Id": "1", "MessageId": "m-1", "SequenceNumber": 1},
            // A duplicate of entry 1 answers with the original message.
            {"Id": "3", "MessageId": "m-1", "SequenceNumber": 1},
            {"Id": "7", "MessageId": "m-2", "SequenceNumber": 2},
        ])
    );
}

#[test]
fn send_batch_applies_max_message_bytes_per_entry() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({"MaxMessageBytes": 2}));
    let r = h.ok(send_batch(
        "q",
        json!([{"Id": "a", "Body": "ab"}, {"Id": "b", "Body": "abc"}]),
    ));
    assert_eq!(failed_codes(&r), vec![("b", "InvalidParameterValue")]);
    assert_eq!(r["Successful"], json!([{"Id": "a", "MessageId": "m-1"}]));
}

#[test]
fn delete_batch_uses_handles_once_and_reports_each_failure() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    h.ok(json!({"Command": "SendMessage", "QueueName": "q", "Body": "x"}));
    h.ok(json!({"Command": "SendMessage", "QueueName": "q", "Body": "y"}));
    h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 2}));
    let r = h.ok(delete_batch(
        "q",
        json!([
            {"Id": "a", "ReceiptHandle": "r-1"},
            {"Id": "b", "ReceiptHandle": "r-1"},
            {"Id": "c", "ReceiptHandle": "r-99"},
            {"Id": "d"},
            {"Id": "e", "ReceiptHandle": 1},
            {"Id": "f", "ReceiptHandle": "r-2"},
        ]),
    ));
    assert_eq!(
        failed_codes(&r),
        vec![
            ("b", "ReceiptHandleExpired"),
            ("c", "ReceiptHandleInvalid"),
            ("d", "MissingParameter"),
            ("e", "InvalidParameterValue")
        ]
    );
    assert_eq!(r["Successful"], json!([{"Id": "a"}, {"Id": "f"}]));
    let c = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(
        c["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn batch_level_rules_fail_the_whole_request_without_effect() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let good = json!({"Id": "a", "Body": "x"});
    let eleven: Vec<Value> = (0..11)
        .map(|i| json!({"Id": format!("i{i}"), "Body": "x"}))
        .collect();
    let ten: Vec<Value> = eleven[..10].to_vec();
    let cases = [
        (json!([]), "EmptyBatchRequest"),
        (json!(eleven), "TooManyEntriesInBatchRequest"),
        (json!([good, 5]), "InvalidBatchEntryId"),
        (json!([good, {"Body": "x"}]), "InvalidBatchEntryId"),
        (
            json!([good, {"Id": null, "Body": "x"}]),
            "InvalidBatchEntryId",
        ),
        (json!([good, {"Id": 1, "Body": "x"}]), "InvalidBatchEntryId"),
        (
            json!([good, {"Id": "", "Body": "x"}]),
            "InvalidBatchEntryId",
        ),
        (
            json!([good, {"Id": "a b", "Body": "x"}]),
            "InvalidBatchEntryId",
        ),
        (
            json!([good, {"Id": "a".repeat(81), "Body": "x"}]),
            "InvalidBatchEntryId",
        ),
        (
            json!([good, {"Id": "a", "Body": "y"}]),
            "BatchEntryIdsNotDistinct",
        ),
        // Rule 3 is checked before rule 4.
        (
            json!([good, {"Id": "a"}, {"Id": "!"}]),
            "InvalidBatchEntryId",
        ),
    ];
    for (entries, code) in cases {
        assert_eq!(h.err(send_batch("q", entries.clone())), code, "{entries}");
        assert_eq!(h.err(delete_batch("q", entries.clone())), code, "{entries}");
    }
    let r = h.ok(send_batch("q", json!(ten)));
    assert_eq!(r["Successful"].as_array().unwrap().len(), 10);
    h.ok(send_batch(
        "q",
        json!([{"Id": "a".repeat(80), "Body": "x"}]),
    ));
    let c = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(
        c["Counts"],
        json!({"Visible": 11, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn batch_check_order_is_required_then_types_then_queue_then_rules() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    for command in ["SendMessageBatch", "DeleteMessageBatch"] {
        assert_eq!(
            h.err(json!({"Command": command, "Entries": []})),
            "MissingParameter"
        );
        assert_eq!(
            h.err(json!({"Command": command, "QueueName": "q"})),
            "MissingParameter"
        );
        assert_eq!(
            h.err(json!({"Command": command, "QueueName": "q", "Entries": null})),
            "MissingParameter"
        );
        assert_eq!(
            h.err(json!({"Command": command, "QueueName": "q", "Entries": {}})),
            "InvalidParameterValue"
        );
        assert_eq!(
            h.err(json!({"Command": command, "QueueName": 1, "Entries": []})),
            "InvalidParameterValue"
        );
        assert_eq!(
            h.err(json!({"Command": command, "QueueName": "nope", "Entries": []})),
            "QueueDoesNotExist"
        );
    }
}
