mod common;

use common::Harness;
use serde_json::{Value, json};

fn create(h: &mut Harness, name: &str, attributes: Value) {
    h.ok(json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attributes}));
}

fn send_batch(h: &mut Harness, queue: &str, entries: Value) -> Value {
    h.ok(json!({"Command": "SendMessageBatch", "QueueName": queue, "Entries": entries}))
}

fn receive(h: &mut Harness, queue: &str, max: u64) -> Vec<Value> {
    let r = h.ok(json!({"Command": "ReceiveMessage", "QueueName": queue, "MaxMessages": max}));
    r["Messages"].as_array().unwrap().clone()
}

fn ids(n: usize) -> Value {
    Value::Array(
        (0..n)
            .map(|i| json!({"Id": format!("e{i}"), "Body": "x"}))
            .collect(),
    )
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
        r,
        json!({"ok": true, "Failed": [], "Successful": [
            {"Id": "a", "MessageId": "m-1"}, {"Id": "b", "MessageId": "m-2"}]})
    );
    let messages = receive(&mut h, "q", 10);
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["Body"], "one");
}

#[test]
fn failed_entries_are_reported_and_do_not_stop_the_others() {
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
            {"Id": "e", "Body": "x", "DelaySeconds": 901},
            {"Id": "f", "Body": "fine"},
        ]),
    );
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
            ("b", "MissingParameter"),
            ("c", "InvalidParameterValue"),
            ("d", "UnsupportedOperation"),
            ("e", "InvalidParameterValue"),
        ]
    );
    assert!(r["Failed"][0]["error"]["message"].is_string());
    // Failed entries use no message id.
    assert_eq!(
        r["Successful"],
        json!([{"Id": "a", "MessageId": "m-1"}, {"Id": "f", "MessageId": "m-2"}])
    );
}

#[test]
fn whole_batch_failing_is_still_a_success() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let r = send_batch(&mut h, "q", json!([{"Id": "a"}]));
    assert_eq!(r["Successful"], json!([]));
    assert_eq!(r["Failed"].as_array().unwrap().len(), 1);
}

#[test]
fn entries_see_earlier_entries_on_fifo_queues() {
    let mut h = Harness::new();
    create(&mut h, "q.fifo", json!({"Fifo": true}));
    let r = send_batch(
        &mut h,
        "q.fifo",
        json!([
            {"Id": "a", "Body": "x", "GroupId": "g", "DeduplicationId": "d"},
            {"Id": "b", "Body": "y", "GroupId": "g", "DeduplicationId": "d"},
            {"Id": "c", "Body": "z", "GroupId": "g", "DeduplicationId": "e"},
            {"Id": "d", "Body": "z", "GroupId": "g"},
            {"Id": "e", "Body": "z", "GroupId": "g", "DelaySeconds": 1, "DeduplicationId": "f"},
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
    assert_eq!(r["Failed"][1]["error"]["code"], "UnsupportedOperation");
}

#[test]
fn max_message_bytes_applies_per_entry() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({"MaxMessageBytes": 2}));
    let r = send_batch(
        &mut h,
        "q",
        json!([{"Id": "a", "Body": "ab"}, {"Id": "b", "Body": "abc"}]),
    );
    assert_eq!(r["Successful"], json!([{"Id": "a", "MessageId": "m-1"}]));
    assert_eq!(r["Failed"][0]["error"]["code"], "InvalidParameterValue");
}

#[test]
fn message_limit_is_seen_by_later_entries() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    for _ in 0..10_000 {
        send_batch(&mut h, "q", ids(10));
    }
    let r = send_batch(&mut h, "q", ids(2));
    assert_eq!(r["Successful"], json!([]));
    assert_eq!(r["Failed"][1]["error"]["code"], "LimitExceeded");
}

#[test]
fn batch_shape_rules() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let send = |entries: Value| json!({"Command": "SendMessageBatch", "QueueName": "q", "Entries": entries});
    assert_eq!(h.err(send(json!([]))), "EmptyBatchRequest");
    assert_eq!(h.err(send(ids(11))), "TooManyEntriesInBatchRequest");
    assert_eq!(h.run(send(ids(10)))["ok"], true);
    for bad in [
        json!(["a"]),
        json!([null]),
        json!([{"Body": "x"}]),
        json!([{"Id": null, "Body": "x"}]),
        json!([{"Id": 1, "Body": "x"}]),
        json!([{"Id": "", "Body": "x"}]),
        json!([{"Id": "a b", "Body": "x"}]),
        json!([{"Id": "i".repeat(81), "Body": "x"}]),
    ] {
        assert_eq!(h.err(send(bad.clone())), "InvalidBatchEntryId", "{bad}");
    }
    assert_eq!(
        h.run(send(json!([{"Id": "i".repeat(80), "Body": "x"}])))["ok"],
        true
    );
    let dup = json!([{"Id": "a", "Body": "x"}, {"Id": "a", "Body": "y"}]);
    assert_eq!(h.err(send(dup)), "BatchEntryIdsNotDistinct");
}

#[test]
fn shape_rules_apply_in_order_and_do_nothing() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let send = |entries: Value| json!({"Command": "SendMessageBatch", "QueueName": "q", "Entries": entries});
    // Too many entries wins over invalid ids, an invalid id over duplicates.
    let mut many = ids(11).as_array().unwrap().clone();
    many[3] = json!(7);
    assert_eq!(
        h.err(send(Value::Array(many))),
        "TooManyEntriesInBatchRequest"
    );
    let both = json!([{"Id": "a", "Body": "x"}, {"Id": "a", "Body": "x"}, 5]);
    assert_eq!(h.err(send(both)), "InvalidBatchEntryId");
    let r = send_batch(&mut h, "q", json!([{"Id": "a", "Body": "x"}]));
    assert_eq!(r["Successful"][0]["MessageId"], "m-1");
}

#[test]
fn request_level_check_order() {
    let mut h = Harness::new();
    let mut c = |v: Value| h.err(v);
    assert_eq!(
        c(json!({"Command": "SendMessageBatch", "Entries": []})),
        "MissingParameter"
    );
    assert_eq!(
        c(json!({"Command": "SendMessageBatch", "QueueName": "q"})),
        "MissingParameter"
    );
    assert_eq!(
        c(json!({"Command": "SendMessageBatch", "QueueName": "q", "Entries": null})),
        "MissingParameter"
    );
    assert_eq!(
        c(json!({"Command": "SendMessageBatch", "QueueName": 1, "Entries": []})),
        "InvalidParameterValue"
    );
    assert_eq!(
        c(json!({"Command": "SendMessageBatch", "QueueName": "q", "Entries": {}})),
        "InvalidParameterValue"
    );
    // The queue lookup comes before the numbered batch rules.
    assert_eq!(
        c(json!({"Command": "SendMessageBatch", "QueueName": "q", "Entries": []})),
        "QueueDoesNotExist"
    );
    assert_eq!(
        c(json!({"Command": "DeleteMessageBatch", "QueueName": "q", "Entries": []})),
        "QueueDoesNotExist"
    );
}

#[test]
fn delete_batch_handles_entries_one_by_one() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    send_batch(&mut h, "q", ids(2));
    let got = receive(&mut h, "q", 2);
    let (r1, r2) = (
        got[0]["ReceiptHandle"].clone(),
        got[1]["ReceiptHandle"].clone(),
    );
    let r = h.ok(
        json!({"Command": "DeleteMessageBatch", "QueueName": "q", "Entries": [
            {"Id": "a", "ReceiptHandle": r1},
            {"Id": "b", "ReceiptHandle": r1},
            {"Id": "c", "ReceiptHandle": "r-99"},
            {"Id": "d"},
            {"Id": "e", "ReceiptHandle": r2},
        ]}),
    );
    assert_eq!(r["Successful"], json!([{"Id": "a"}, {"Id": "e"}]));
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
            ("b", "ReceiptHandleExpired"),
            ("c", "ReceiptHandleInvalid"),
            ("d", "MissingParameter"),
        ]
    );
    let counts = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}))["Counts"].clone();
    assert_eq!(counts, json!({"Visible": 0, "InFlight": 0, "Delayed": 0}));
}

#[test]
fn delete_batch_ignores_other_members_and_null() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    send_batch(&mut h, "q", ids(1));
    let got = receive(&mut h, "q", 1);
    let r = h.ok(
        json!({"Command": "DeleteMessageBatch", "QueueName": "q", "Entries": [
        {"Id": "a", "ReceiptHandle": got[0]["ReceiptHandle"], "Body": 5, "X": null}]}),
    );
    assert_eq!(r["Successful"], json!([{"Id": "a"}]));
    let r = h.ok(
        json!({"Command": "DeleteMessageBatch", "QueueName": "q", "Entries": [
        {"Id": "a", "ReceiptHandle": null}]}),
    );
    assert_eq!(r["Failed"][0]["error"]["code"], "MissingParameter");
}

#[test]
fn delete_batch_shape_errors() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let del = |entries: Value| json!({"Command": "DeleteMessageBatch", "QueueName": "q", "Entries": entries});
    assert_eq!(h.err(del(json!([]))), "EmptyBatchRequest");
    assert_eq!(h.err(del(ids(11))), "TooManyEntriesInBatchRequest");
    assert_eq!(
        h.err(del(json!([{"ReceiptHandle": "r-1"}]))),
        "InvalidBatchEntryId"
    );
    let dup = json!([{"Id": "a", "ReceiptHandle": "r-1"}, {"Id": "a", "ReceiptHandle": "r-2"}]);
    assert_eq!(h.err(del(dup)), "BatchEntryIdsNotDistinct");
}
