mod common;

use common::Harness;
use serde_json::{Value, json};

fn queue(h: &mut Harness, name: &str, attributes: Value) {
    h.ok(json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attributes}));
}

fn send_batch(h: &mut Harness, queue: &str, entries: Value) -> Value {
    h.ok(json!({"Command": "SendMessageBatch", "QueueName": queue, "Entries": entries}))
}

fn counts(h: &mut Harness, queue: &str) -> Value {
    h.ok(json!({"Command": "GetQueueAttributes", "QueueName": queue}))["Counts"].clone()
}

fn receive(h: &mut Harness, queue: &str, max: u64) -> Vec<Value> {
    let r = h.ok(json!({"Command": "ReceiveMessage", "QueueName": queue, "MaxMessages": max}));
    r["Messages"].as_array().unwrap().clone()
}

fn failed_codes(response: &Value) -> Vec<(String, String)> {
    response["Failed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            (
                f["Id"].as_str().unwrap().to_owned(),
                f["error"]["code"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

#[test]
fn send_batch_succeeds_in_order_with_message_ids() {
    let mut h = Harness::new();
    queue(&mut h, "q", json!({}));
    let r = send_batch(
        &mut h,
        "q",
        json!([{"Id": "a", "Body": "one"}, {"Id": "b", "Body": "two", "DelaySeconds": 5}]),
    );
    assert_eq!(
        r,
        json!({"ok": true, "Failed": [],
               "Successful": [{"Id": "a", "MessageId": "m-1"}, {"Id": "b", "MessageId": "m-2"}]})
    );
    assert_eq!(
        counts(&mut h, "q"),
        json!({"Visible": 1, "InFlight": 0, "Delayed": 1})
    );
}

#[test]
fn failed_entries_do_not_stop_others_or_use_ids() {
    let mut h = Harness::new();
    queue(&mut h, "q", json!({"MaxMessageBytes": 3}));
    let r = send_batch(
        &mut h,
        "q",
        json!([
            {"Id": "1", "Body": "abcd"},
            {"Id": "2"},
            {"Id": "3", "Body": "ok"},
            {"Id": "4", "Body": "ok", "GroupId": "g"},
            {"Id": "5", "Body": "x", "DelaySeconds": 901},
            {"Id": "6", "Body": null},
            {"Id": "7", "Body": "fine", "Extra": 1},
        ]),
    );
    assert_eq!(r["ok"], true);
    assert_eq!(
        failed_codes(&r),
        [
            ("1", "InvalidParameterValue"),
            ("2", "MissingParameter"),
            ("4", "UnsupportedOperation"),
            ("5", "InvalidParameterValue"),
            ("6", "MissingParameter"),
            ("7", "InvalidParameterValue"),
        ]
        .map(|(a, b)| (a.to_owned(), b.to_owned()))
    );
    assert!(r["Failed"][0]["error"]["message"].is_string());
    assert_eq!(r["Successful"], json!([{"Id": "3", "MessageId": "m-1"}]));
    assert_eq!(counts(&mut h, "q")["Visible"], 1);
}

#[test]
fn all_entries_failing_is_still_a_success() {
    let mut h = Harness::new();
    queue(&mut h, "q", json!({}));
    let r = send_batch(&mut h, "q", json!([{"Id": "a"}]));
    assert_eq!(r["Successful"], json!([]));
    assert_eq!(
        failed_codes(&r),
        [("a".to_owned(), "MissingParameter".to_owned())]
    );
}

#[test]
fn fifo_entries_see_earlier_entries() {
    let mut h = Harness::new();
    queue(&mut h, "q.fifo", json!({"Fifo": true}));
    let r = send_batch(
        &mut h,
        "q.fifo",
        json!([
            {"Id": "a", "Body": "x", "GroupId": "g", "DeduplicationId": "d"},
            {"Id": "b", "Body": "y", "GroupId": "g", "DeduplicationId": "d"},
            {"Id": "c", "Body": "z", "GroupId": "g", "DeduplicationId": "e"},
            {"Id": "d", "Body": "z", "DeduplicationId": "f"},
            {"Id": "e", "Body": "z", "GroupId": "g", "DelaySeconds": 1, "DeduplicationId": "g"},
        ]),
    );
    // The duplicate answers with the original's ids; no number is used up.
    assert_eq!(
        r["Successful"],
        json!([
            {"Id": "a", "MessageId": "m-1", "SequenceNumber": 1},
            {"Id": "b", "MessageId": "m-1", "SequenceNumber": 1},
            {"Id": "c", "MessageId": "m-2", "SequenceNumber": 2},
        ])
    );
    assert_eq!(
        failed_codes(&r),
        [("d", "MissingParameter"), ("e", "UnsupportedOperation")]
            .map(|(a, b)| (a.to_owned(), b.to_owned()))
    );
}

#[test]
fn content_based_deduplication_in_a_batch() {
    let mut h = Harness::new();
    queue(
        &mut h,
        "q.fifo",
        json!({"Fifo": true, "ContentBasedDeduplication": true}),
    );
    let r = send_batch(
        &mut h,
        "q.fifo",
        json!([{"Id": "a", "Body": "same", "GroupId": "g"}, {"Id": "b", "Body": "same", "GroupId": "h"}]),
    );
    assert_eq!(
        r["Successful"][1],
        json!({"Id": "b", "MessageId": "m-1", "SequenceNumber": 1})
    );
}

#[test]
fn batch_validation_order() {
    let mut h = Harness::new();
    queue(&mut h, "q", json!({}));
    let s = |entries: Value| json!({"Command": "SendMessageBatch", "QueueName": "q", "Entries": entries});
    assert_eq!(h.err(s(json!([]))), "EmptyBatchRequest");
    let eleven: Vec<Value> = (0..11)
        .map(|i| json!({"Id": format!("i{i}"), "Body": "x"}))
        .collect();
    assert_eq!(
        h.err(s(Value::Array(eleven))),
        "TooManyEntriesInBatchRequest"
    );
    // Rule 2 comes before rule 3: eleven bad entries are "too many".
    assert_eq!(
        h.err(s(Value::Array(vec![json!(1); 11]))),
        "TooManyEntriesInBatchRequest"
    );
    for bad in [
        json!(["x"]),
        json!([{"Body": "x"}]),
        json!([{"Id": null, "Body": "x"}]),
        json!([{"Id": 1, "Body": "x"}]),
        json!([{"Id": "", "Body": "x"}]),
        json!([{"Id": "a b", "Body": "x"}]),
        json!([{"Id": "a".repeat(81), "Body": "x"}]),
        json!([{"Id": "ok", "Body": "x"}, null]),
    ] {
        assert_eq!(h.err(s(bad)), "InvalidBatchEntryId");
    }
    // Rule 3 comes before rule 4.
    assert_eq!(
        h.err(s(json!([{"Id": "a"}, {"Id": "a"}, {"Id": 5}]))),
        "InvalidBatchEntryId"
    );
    assert_eq!(
        h.err(s(
            json!([{"Id": "a", "Body": "x"}, {"Id": "a", "Body": "y"}])
        )),
        "BatchEntryIdsNotDistinct"
    );
    // Ids are case-sensitive; 80 characters are fine.
    let ok = send_batch(
        &mut h,
        "q",
        json!([{"Id": "a", "Body": "x"}, {"Id": "A", "Body": "x"}, {"Id": "z".repeat(80), "Body": "x"}]),
    );
    assert_eq!(ok["Successful"].as_array().unwrap().len(), 3);
    // Ten entries are allowed.
    let ten: Vec<Value> = (0..10)
        .map(|i| json!({"Id": format!("i{i}"), "Body": "x"}))
        .collect();
    assert_eq!(
        send_batch(&mut h, "q", Value::Array(ten))["Successful"]
            .as_array()
            .unwrap()
            .len(),
        10
    );
    // Nothing from the rejected requests was sent.
    assert_eq!(counts(&mut h, "q")["Visible"], 13);
}

#[test]
fn earlier_checks_come_first() {
    let mut h = Harness::new();
    for cmd in ["SendMessageBatch", "DeleteMessageBatch"] {
        assert_eq!(
            h.err(json!({"Command": cmd, "Entries": []})),
            "MissingParameter"
        );
        assert_eq!(
            h.err(json!({"Command": cmd, "QueueName": "q"})),
            "MissingParameter"
        );
        assert_eq!(
            h.err(json!({"Command": cmd, "QueueName": "q", "Entries": null})),
            "MissingParameter"
        );
        assert_eq!(
            h.err(json!({"Command": cmd, "QueueName": 1, "Entries": []})),
            "InvalidParameterValue"
        );
        assert_eq!(
            h.err(json!({"Command": cmd, "QueueName": "q", "Entries": {}})),
            "InvalidParameterValue"
        );
        assert_eq!(
            h.err(json!({"Command": cmd, "QueueName": "q", "Entries": "x"})),
            "InvalidParameterValue"
        );
        // The queue lookup precedes the batch rules.
        assert_eq!(
            h.err(json!({"Command": cmd, "QueueName": "q", "Entries": []})),
            "QueueDoesNotExist"
        );
    }
}

#[test]
fn message_limit_applies_within_a_batch() {
    let mut h = Harness::new();
    queue(&mut h, "q", json!({}));
    for _ in 0..10_000 {
        let entries: Vec<Value> = (0..10)
            .map(|i| json!({"Id": format!("i{i}"), "Body": "x"}))
            .collect();
        send_batch(&mut h, "q", Value::Array(entries));
    }
    let r = send_batch(&mut h, "q", json!([{"Id": "a", "Body": "x"}]));
    assert_eq!(
        failed_codes(&r),
        [("a".to_owned(), "LimitExceeded".to_owned())]
    );
    assert_eq!(r["Successful"], json!([]));
}

#[test]
fn delete_batch_uses_handles_and_sees_earlier_entries() {
    let mut h = Harness::new();
    queue(&mut h, "q", json!({}));
    queue(&mut h, "other", json!({}));
    send_batch(
        &mut h,
        "q",
        json!([{"Id": "a", "Body": "1"}, {"Id": "b", "Body": "2"}, {"Id": "c", "Body": "3"}]),
    );
    h.ok(json!({"Command": "SendMessage", "QueueName": "other", "Body": "o"}));
    let got = receive(&mut h, "q", 3);
    let handles: Vec<&str> = got
        .iter()
        .map(|m| m["ReceiptHandle"].as_str().unwrap())
        .collect();
    assert_eq!(handles, ["r-1", "r-2", "r-3"]);
    let foreign = receive(&mut h, "other", 1);
    assert_eq!(foreign[0]["ReceiptHandle"], "r-4");

    let r = h.ok(
        json!({"Command": "DeleteMessageBatch", "QueueName": "q", "Entries": [
            {"Id": "e1", "ReceiptHandle": "r-1"},
            {"Id": "e2", "ReceiptHandle": "r-1"},
            {"Id": "e3", "ReceiptHandle": "r-99"},
            {"Id": "e4", "ReceiptHandle": "r-04"},
            {"Id": "e5"},
            {"Id": "e6", "ReceiptHandle": 5},
            {"Id": "e7", "ReceiptHandle": "r-4"},
            {"Id": "e8", "ReceiptHandle": "r-3", "Body": "ignored"},
        ]}),
    );
    assert_eq!(r["Successful"], json!([{"Id": "e1"}, {"Id": "e8"}]));
    assert_eq!(
        failed_codes(&r),
        [
            ("e2", "ReceiptHandleExpired"),
            ("e3", "ReceiptHandleInvalid"),
            ("e4", "ReceiptHandleInvalid"),
            ("e5", "MissingParameter"),
            ("e6", "InvalidParameterValue"),
            ("e7", "ReceiptHandleExpired"),
        ]
        .map(|(a, b)| (a.to_owned(), b.to_owned()))
    );
    assert!(r["Failed"][0]["error"]["message"].is_string());
    assert_eq!(
        counts(&mut h, "q"),
        json!({"Visible": 0, "InFlight": 1, "Delayed": 0})
    );
    assert_eq!(counts(&mut h, "other")["InFlight"], 1);
}

#[test]
fn delete_batch_with_only_failures_is_a_success() {
    let mut h = Harness::new();
    queue(&mut h, "q", json!({}));
    let r = h.ok(json!({"Command": "DeleteMessageBatch", "QueueName": "q",
                        "Entries": [{"Id": "x", "ReceiptHandle": "r-1"}]}));
    assert_eq!(r["Successful"], json!([]));
    assert_eq!(
        failed_codes(&r),
        [("x".to_owned(), "ReceiptHandleInvalid".to_owned())]
    );
}

#[test]
fn delete_batch_rejects_bad_ids_without_deleting() {
    let mut h = Harness::new();
    queue(&mut h, "q", json!({}));
    h.ok(json!({"Command": "SendMessage", "QueueName": "q", "Body": "x"}));
    receive(&mut h, "q", 1);
    let d = |entries: Value| json!({"Command": "DeleteMessageBatch", "QueueName": "q", "Entries": entries});
    assert_eq!(h.err(d(json!([]))), "EmptyBatchRequest");
    assert_eq!(
        h.err(d(json!([{"ReceiptHandle": "r-1"}]))),
        "InvalidBatchEntryId"
    );
    assert_eq!(
        h.err(d(
            json!([{"Id": "a", "ReceiptHandle": "r-1"}, {"Id": "a", "ReceiptHandle": "r-1"}])
        )),
        "BatchEntryIdsNotDistinct"
    );
    let eleven: Vec<Value> = (0..11)
        .map(|i| json!({"Id": format!("i{i}"), "ReceiptHandle": "r-1"}))
        .collect();
    assert_eq!(
        h.err(d(Value::Array(eleven))),
        "TooManyEntriesInBatchRequest"
    );
    assert_eq!(counts(&mut h, "q")["InFlight"], 1);
}
