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

fn receive(h: &mut Harness, queue: &str, max: u64) -> Vec<Value> {
    let r = h.ok(json!({"Command": "ReceiveMessage", "QueueName": queue, "MaxMessages": max}));
    r["Messages"].as_array().unwrap().clone()
}

fn codes(failed: &Value) -> Vec<(&str, &str)> {
    failed
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            assert!(f["error"]["message"].is_string());
            (
                f["Id"].as_str().unwrap(),
                f["error"]["code"].as_str().unwrap(),
            )
        })
        .collect()
}

#[test]
fn send_batch_results_follow_entry_order() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let r = send_batch(
        &mut h,
        "q",
        json!([{"Id": "a", "Body": "one"}, {"Id": "b", "Body": "two", "DelaySeconds": 5}]),
    );
    assert_eq!(
        r["Successful"],
        json!([{"Id": "a", "MessageId": "m-1"}, {"Id": "b", "MessageId": "m-2"}])
    );
    assert_eq!(r["Failed"], json!([]));
    let bodies: Vec<_> = receive(&mut h, "q", 10)
        .iter()
        .map(|m| m["Body"].clone())
        .collect();
    assert_eq!(bodies, [json!("one")]);
}

#[test]
fn failed_entries_do_not_stop_the_others_or_use_ids() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({"MaxMessageBytes": 3}));
    let r = send_batch(
        &mut h,
        "q",
        json!([
            {"Id": "1", "Body": "ok"},
            {"Id": "2"},
            {"Id": "3", "Body": "toolong"},
            {"Id": "4", "Body": "x", "DelaySeconds": 901},
            {"Id": "5", "Body": "y", "GroupId": "g"},
            {"Id": "6", "Body": null},
            {"Id": "7", "Body": "z", "Extra": true, "QueueName": "ignored"},
        ]),
    );
    assert_eq!(
        r["Successful"],
        json!([{"Id": "1", "MessageId": "m-1"}, {"Id": "7", "MessageId": "m-2"}])
    );
    assert_eq!(
        codes(&r["Failed"]),
        [
            ("2", "MissingParameter"),
            ("3", "InvalidParameterValue"),
            ("4", "InvalidParameterValue"),
            ("5", "UnsupportedOperation"),
            ("6", "MissingParameter"),
        ]
    );
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
            {"Id": "c", "Body": "z", "GroupId": "g"},
            {"Id": "d", "Body": "w", "GroupId": "g", "DeduplicationId": "e"},
        ]),
    );
    assert_eq!(
        r["Successful"],
        json!([
            {"Id": "a", "MessageId": "m-1", "SequenceNumber": 1},
            {"Id": "b", "MessageId": "m-1", "SequenceNumber": 1},
            {"Id": "d", "MessageId": "m-2", "SequenceNumber": 2},
        ])
    );
    assert_eq!(codes(&r["Failed"]), [("c", "MissingParameter")]);
}

#[test]
fn message_limit_applies_within_a_batch() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    for _ in 0..10_000 {
        let entries: Vec<Value> = (0..10)
            .map(|i| json!({"Id": format!("i{i}"), "Body": "x"}))
            .collect();
        let r = send_batch(&mut h, "q", json!(entries));
        assert_eq!(r["Successful"].as_array().unwrap().len(), 10);
    }
    let r = send_batch(&mut h, "q", json!([{"Id": "a", "Body": "x"}]));
    assert_eq!(codes(&r["Failed"]), [("a", "LimitExceeded")]);
    assert_eq!(r["Successful"], json!([]));
}

#[test]
fn delete_batch_handles_each_handle_like_delete_message() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    create(&mut h, "other", json!({}));
    send_batch(
        &mut h,
        "q",
        json!([{"Id": "a", "Body": "1"}, {"Id": "b", "Body": "2"}]),
    );
    send_batch(&mut h, "other", json!([{"Id": "a", "Body": "3"}]));
    let got = receive(&mut h, "q", 2);
    let foreign = receive(&mut h, "other", 1);
    let (r1, r2) = (
        got[0]["ReceiptHandle"].clone(),
        got[1]["ReceiptHandle"].clone(),
    );
    let r = delete_batch(
        &mut h,
        "q",
        json!([
            {"Id": "x", "ReceiptHandle": r1},
            {"Id": "y", "ReceiptHandle": r1},
            {"Id": "z", "ReceiptHandle": "r-99"},
            {"Id": "w", "ReceiptHandle": foreign[0]["ReceiptHandle"]},
            {"Id": "v"},
            {"Id": "u", "ReceiptHandle": 5},
            {"Id": "t", "ReceiptHandle": r2},
        ]),
    );
    assert_eq!(r["Successful"], json!([{"Id": "x"}, {"Id": "t"}]));
    assert_eq!(
        codes(&r["Failed"]),
        [
            ("y", "ReceiptHandleExpired"),
            ("z", "ReceiptHandleInvalid"),
            ("w", "ReceiptHandleExpired"),
            ("v", "MissingParameter"),
            ("u", "InvalidParameterValue"),
        ]
    );
    let counts = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}))["Counts"].clone();
    assert_eq!(counts, json!({"Visible": 0, "InFlight": 0, "Delayed": 0}));
}

#[test]
fn request_level_failures_do_nothing() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let eleven: Vec<Value> = (0..11)
        .map(|i| json!({"Id": format!("i{i}"), "Body": "x"}))
        .collect();
    let max_id = "a".repeat(80);
    let long_id = "a".repeat(81);
    let cases = [
        (json!([]), "EmptyBatchRequest"),
        (json!(eleven), "TooManyEntriesInBatchRequest"),
        (json!(["x"]), "InvalidBatchEntryId"),
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
        // Rule 3 is checked for all entries before rule 4.
        (json!([{"Id": "a"}, {"Id": "a"}, 1]), "InvalidBatchEntryId"),
        (json!("x"), "InvalidParameterValue"),
        (json!({"Id": "a"}), "InvalidParameterValue"),
    ];
    for (entries, code) in cases {
        let c = json!({"Command": "SendMessageBatch", "QueueName": "q", "Entries": entries});
        assert_eq!(h.err(c), code, "{entries}");
        let c = json!({"Command": "DeleteMessageBatch", "QueueName": "q", "Entries": entries});
        assert_eq!(h.err(c), code, "{entries}");
    }
    let r = send_batch(&mut h, "q", json!([{"Id": max_id, "Body": "x"}]));
    assert_eq!(
        r["Successful"],
        json!([{"Id": "a".repeat(80), "MessageId": "m-1"}])
    );
}

#[test]
fn batch_check_order() {
    let mut h = Harness::new();
    for command in ["SendMessageBatch", "DeleteMessageBatch"] {
        assert_eq!(
            h.err(json!({"Command": command, "QueueName": "q"})),
            "MissingParameter"
        );
        assert_eq!(
            h.err(json!({"Command": command, "Entries": []})),
            "MissingParameter"
        );
        // Value checks come before the queue lookup, which comes before the rules.
        assert_eq!(
            h.err(json!({"Command": command, "QueueName": 1, "Entries": []})),
            "InvalidParameterValue"
        );
        assert_eq!(
            h.err(json!({"Command": command, "QueueName": "q", "Entries": 1})),
            "InvalidParameterValue"
        );
        assert_eq!(
            h.err(json!({"Command": command, "QueueName": "q", "Entries": []})),
            "QueueDoesNotExist"
        );
    }
}
