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

fn counts(h: &mut Harness, queue: &str) -> Value {
    h.ok(json!({"Command": "GetQueueAttributes", "QueueName": queue}))["Counts"].clone()
}

fn error_codes(response: &Value) -> Vec<(&str, &str)> {
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
fn send_batch_accepts_entries_in_order() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let r = h.ok(send_batch(
        "q",
        json!([{"Id": "a", "Body": "x"}, {"Id": "b", "Body": "y", "DelaySeconds": 5}]),
    ));
    assert_eq!(
        r["Successful"],
        json!([{"Id": "a", "MessageId": "m-1"}, {"Id": "b", "MessageId": "m-2"}])
    );
    assert_eq!(r["Failed"], json!([]));
    assert_eq!(
        counts(&mut h, "q"),
        json!({"Visible": 1, "InFlight": 0, "Delayed": 1})
    );
}

#[test]
fn send_batch_fifo_returns_sequence_numbers_and_sees_earlier_entries() {
    let mut h = Harness::new();
    create(&mut h, "q.fifo", json!({"Fifo": true}));
    let r = h.ok(send_batch(
        "q.fifo",
        json!([
            {"Id": "a", "Body": "x", "GroupId": "g", "DeduplicationId": "d"},
            {"Id": "b", "Body": "y", "GroupId": "g", "DeduplicationId": "d"},
            {"Id": "c", "Body": "z", "GroupId": "g", "DeduplicationId": "e"},
        ]),
    ));
    assert_eq!(
        r["Successful"],
        json!([
            {"Id": "a", "MessageId": "m-1", "SequenceNumber": 1},
            {"Id": "b", "MessageId": "m-1", "SequenceNumber": 1},
            {"Id": "c", "MessageId": "m-2", "SequenceNumber": 2},
        ])
    );
}

#[test]
fn failed_entries_do_not_stop_the_others_or_use_ids() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({"MaxMessageBytes": 3}));
    let r = h.ok(send_batch(
        "q",
        json!([
            {"Id": "a", "Body": "1"},
            {"Id": "b"},
            {"Id": "c", "Body": "toolong"},
            {"Id": "d", "Body": "x", "DelaySeconds": 901},
            {"Id": "e", "Body": 5},
            {"Id": "f", "Body": "", "Unknown": 1},
            {"Id": "g", "Body": "x", "GroupId": "g"},
            {"Id": "h", "Body": "2", "DelaySeconds": null},
        ]),
    ));
    assert_eq!(
        r["Successful"],
        json!([{"Id": "a", "MessageId": "m-1"}, {"Id": "h", "MessageId": "m-2"}])
    );
    assert_eq!(
        error_codes(&r),
        vec![
            ("b", "MissingParameter"),
            ("c", "InvalidParameterValue"),
            ("d", "InvalidParameterValue"),
            ("e", "InvalidParameterValue"),
            ("f", "InvalidParameterValue"),
            ("g", "UnsupportedOperation"),
        ]
    );
    assert!(r["Failed"][0]["error"]["message"].is_string());
}

#[test]
fn send_batch_hits_the_queue_limit_mid_batch() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    for _ in 0..10_000 {
        let entries: Vec<Value> = (0..10)
            .map(|i| json!({"Id": format!("i{i}"), "Body": "x"}))
            .collect();
        let r = h.ok(send_batch("q", json!(entries)));
        assert_eq!(r["Failed"], json!([]));
    }
    let r = h.ok(send_batch("q", json!([{"Id": "a", "Body": "x"}])));
    assert_eq!(error_codes(&r), vec![("a", "LimitExceeded")]);
}

#[test]
fn batch_shape_errors_in_order() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let eleven: Vec<Value> = (0..11).map(|i| json!({"Id": format!("i{i}")})).collect();
    let cases = [
        (json!([]), "EmptyBatchRequest"),
        (json!(eleven), "TooManyEntriesInBatchRequest"),
        (json!([5]), "InvalidBatchEntryId"),
        (json!([{"Body": "x"}]), "InvalidBatchEntryId"),
        (json!([{"Id": null}]), "InvalidBatchEntryId"),
        (json!([{"Id": 1}]), "InvalidBatchEntryId"),
        (json!([{"Id": ""}]), "InvalidBatchEntryId"),
        (json!([{"Id": "a b"}]), "InvalidBatchEntryId"),
        (json!([{"Id": "x".repeat(81)}]), "InvalidBatchEntryId"),
        (
            json!([{"Id": "a"}, {"Id": "a"}]),
            "BatchEntryIdsNotDistinct",
        ),
        // The id rule is checked on every entry before distinctness.
        (json!([{"Id": "a"}, {"Id": "a"}, 5]), "InvalidBatchEntryId"),
        // Entry contents are not looked at by the shape rules.
        (
            json!([{"Id": "a", "Body": 5}, {"Id": "a", "Body": 5}]),
            "BatchEntryIdsNotDistinct",
        ),
    ];
    for command in [send_batch, delete_batch] {
        for (entries, code) in &cases {
            assert_eq!(h.err(command("q", entries.clone())), *code, "{entries}");
        }
    }
    assert_eq!(
        counts(&mut h, "q"),
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn id_length_limit_is_inclusive() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let id = "x".repeat(80);
    let r = h.ok(send_batch("q", json!([{"Id": id, "Body": "b"}])));
    assert_eq!(r["Successful"][0]["Id"], json!(id));
}

#[test]
fn batch_check_order() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    for command in ["SendMessageBatch", "DeleteMessageBatch"] {
        assert_eq!(h.err(json!({"Command": command})), "MissingParameter");
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
        // The queue lookup comes before the batch rules.
        assert_eq!(
            h.err(json!({"Command": command, "QueueName": "nope", "Entries": []})),
            "QueueDoesNotExist"
        );
    }
}

#[test]
fn delete_batch_removes_messages_and_reports_each_handle() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    create(&mut h, "other", json!({}));
    for body in ["a", "b", "c"] {
        h.ok(json!({"Command": "SendMessage", "QueueName": "q", "Body": body}));
    }
    h.ok(json!({"Command": "SendMessage", "QueueName": "other", "Body": "o"}));
    h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 3}));
    h.ok(json!({"Command": "ReceiveMessage", "QueueName": "other"}));
    let r = h.ok(delete_batch(
        "q",
        json!([
            {"Id": "1", "ReceiptHandle": "r-1"},
            {"Id": "2", "ReceiptHandle": "r-1"},
            {"Id": "3", "ReceiptHandle": "r-99"},
            {"Id": "4"},
            {"Id": "5", "ReceiptHandle": 7},
            {"Id": "6", "ReceiptHandle": "r-4"},
            {"Id": "7", "ReceiptHandle": "r-3", "Body": "ignored"},
        ]),
    ));
    assert_eq!(r["Successful"], json!([{"Id": "1"}, {"Id": "7"}]));
    assert_eq!(
        error_codes(&r),
        vec![
            ("2", "ReceiptHandleExpired"),
            ("3", "ReceiptHandleInvalid"),
            ("4", "MissingParameter"),
            ("5", "InvalidParameterValue"),
            ("6", "ReceiptHandleExpired"),
        ]
    );
    assert_eq!(
        counts(&mut h, "q"),
        json!({"Visible": 0, "InFlight": 1, "Delayed": 0})
    );
    assert_eq!(
        counts(&mut h, "other"),
        json!({"Visible": 0, "InFlight": 1, "Delayed": 0})
    );
}

#[test]
fn ten_entries_are_allowed() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let entries: Vec<Value> = (0..10)
        .map(|i| json!({"Id": format!("i{i}"), "Body": "x"}))
        .collect();
    let r = h.ok(send_batch("q", json!(entries)));
    assert_eq!(r["Successful"].as_array().unwrap().len(), 10);
}
