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

fn receive(h: &mut Harness, queue: &str, max: u64) -> Vec<Value> {
    let r = h.ok(json!({"Command": "ReceiveMessage", "QueueName": queue, "MaxMessages": max}));
    r["Messages"].as_array().unwrap().clone()
}

fn codes(response: &Value) -> Vec<(&str, &str)> {
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
fn send_batch_reports_each_entry_in_order() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let r = send_batch(
        &mut h,
        "q",
        json!([{"Id": "a", "Body": "x"}, {"Id": "b"}, {"Id": "c", "Body": "y", "DelaySeconds": 5},
               {"Id": "d", "Body": "z", "DelaySeconds": 901}, {"Id": "e", "Body": "w", "GroupId": "g"}]),
    );
    assert_eq!(
        r,
        json!({"ok": true,
        "Successful": [{"Id": "a", "MessageId": "m-1"}, {"Id": "c", "MessageId": "m-2"}],
        "Failed": [
            {"Id": "b", "error": {"code": "MissingParameter", "message": r["Failed"][0]["error"]["message"]}},
            {"Id": "d", "error": {"code": "InvalidParameterValue", "message": r["Failed"][1]["error"]["message"]}},
            {"Id": "e", "error": {"code": "UnsupportedOperation", "message": r["Failed"][2]["error"]["message"]}},
        ]})
    );
    let counts = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}))["Counts"].clone();
    assert_eq!(counts, json!({"Visible": 1, "InFlight": 0, "Delayed": 1}));
}

#[test]
fn send_batch_entries_see_earlier_entries_on_a_fifo_queue() {
    let mut h = Harness::new();
    create(&mut h, "q.fifo", json!({"Fifo": true}));
    let r = send_batch(
        &mut h,
        "q.fifo",
        json!([{"Id": "a", "Body": "x", "GroupId": "g", "DeduplicationId": "d"},
               {"Id": "b", "Body": "y", "GroupId": "g", "DeduplicationId": "d"},
               {"Id": "c", "Body": "z", "GroupId": "g", "DeduplicationId": "e"},
               {"Id": "d", "Body": "z", "GroupId": "g"}]),
    );
    assert_eq!(
        r["Successful"],
        json!([{"Id": "a", "MessageId": "m-1", "SequenceNumber": 1},
               {"Id": "b", "MessageId": "m-1", "SequenceNumber": 1},
               {"Id": "c", "MessageId": "m-2", "SequenceNumber": 2}])
    );
    assert_eq!(codes(&r), [("d", "MissingParameter")]);
}

#[test]
fn send_batch_stops_at_the_message_limit_and_the_body_limit() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({"MaxMessageBytes": 2}));
    let r = send_batch(
        &mut h,
        "q",
        json!([{"Id": "a", "Body": "ab"}, {"Id": "b", "Body": "abc"}]),
    );
    assert_eq!(r["Successful"], json!([{"Id": "a", "MessageId": "m-1"}]));
    assert_eq!(codes(&r), [("b", "InvalidParameterValue")]);

    create(&mut h, "full", json!({}));
    for chunk in 0..10_000 {
        let entries: Vec<Value> = (0..10)
            .map(|i| json!({"Id": format!("i{i}"), "Body": "x"}))
            .collect();
        let r = send_batch(&mut h, "full", json!(entries));
        assert_eq!(
            r["Successful"].as_array().unwrap().len(),
            10,
            "chunk {chunk}"
        );
    }
    let r = send_batch(&mut h, "full", json!([{"Id": "a", "Body": "x"}]));
    assert_eq!(r["Successful"], json!([]));
    assert_eq!(codes(&r), [("a", "LimitExceeded")]);
}

#[test]
fn delete_batch_sees_handles_used_by_earlier_entries() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    create(&mut h, "other", json!({}));
    send_batch(
        &mut h,
        "q",
        json!([{"Id": "a", "Body": "x"}, {"Id": "b", "Body": "y"}]),
    );
    h.ok(json!({"Command": "SendMessage", "QueueName": "other", "Body": "o"}));
    let handles: Vec<Value> = receive(&mut h, "q", 2)
        .iter()
        .map(|m| m["ReceiptHandle"].clone())
        .collect();
    let foreign = receive(&mut h, "other", 1)[0]["ReceiptHandle"].clone();
    let r = delete_batch(
        &mut h,
        "q",
        json!([{"Id": "1", "ReceiptHandle": handles[0]}, {"Id": "2", "ReceiptHandle": handles[0]},
               {"Id": "3", "ReceiptHandle": "r-99"}, {"Id": "4"}, {"Id": "5", "ReceiptHandle": 7},
               {"Id": "6", "ReceiptHandle": foreign}, {"Id": "7", "ReceiptHandle": handles[1]}]),
    );
    assert_eq!(r["Successful"], json!([{"Id": "1"}, {"Id": "7"}]));
    assert_eq!(
        codes(&r),
        [
            ("2", "ReceiptHandleExpired"),
            ("3", "ReceiptHandleInvalid"),
            ("4", "MissingParameter"),
            ("5", "InvalidParameterValue"),
            ("6", "ReceiptHandleExpired")
        ]
    );
    let counts = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}))["Counts"].clone();
    assert_eq!(counts, json!({"Visible": 0, "InFlight": 0, "Delayed": 0}));
}

#[test]
fn batch_request_rules_fail_the_whole_request_without_effect() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let long_id = "i".repeat(81);
    let cases = [
        (json!([]), "EmptyBatchRequest"),
        (
            json!(
                (0..11)
                    .map(|i| json!({"Id": format!("{i}"), "Body": "x"}))
                    .collect::<Vec<_>>()
            ),
            "TooManyEntriesInBatchRequest",
        ),
        (json!([{"Id": "a", "Body": "x"}, 5]), "InvalidBatchEntryId"),
        (
            json!([{"Id": "a", "Body": "x"}, {"Body": "x"}]),
            "InvalidBatchEntryId",
        ),
        (
            json!([{"Id": "a", "Body": "x"}, {"Id": null, "Body": "x"}]),
            "InvalidBatchEntryId",
        ),
        (
            json!([{"Id": "a", "Body": "x"}, {"Id": 1, "Body": "x"}]),
            "InvalidBatchEntryId",
        ),
        (
            json!([{"Id": "a", "Body": "x"}, {"Id": "", "Body": "x"}]),
            "InvalidBatchEntryId",
        ),
        (
            json!([{"Id": "a", "Body": "x"}, {"Id": "a b", "Body": "x"}]),
            "InvalidBatchEntryId",
        ),
        (
            json!([{"Id": "a", "Body": "x"}, {"Id": long_id, "Body": "x"}]),
            "InvalidBatchEntryId",
        ),
        (
            json!([{"Id": "a", "Body": "x"}, {"Id": "a", "Body": "y"}]),
            "BatchEntryIdsNotDistinct",
        ),
        (json!([{"Id": "a"}, {"Id": "a"}, 3]), "InvalidBatchEntryId"),
    ];
    for (entries, code) in cases {
        let r = send_batch(&mut h, "q", entries.clone());
        assert_eq!(r["error"]["code"], code, "{entries}");
        let r = delete_batch(&mut h, "q", entries.clone());
        assert_eq!(r["error"]["code"], code, "{entries}");
    }
    let ok = send_batch(&mut h, "q", json!([{"Id": "x".repeat(80), "Body": "m"}]));
    assert_eq!(ok["Successful"][0]["MessageId"], "m-1");
    let ten: Vec<Value> = (0..10)
        .map(|i| json!({"Id": format!("{i}"), "Body": "x"}))
        .collect();
    assert_eq!(
        send_batch(&mut h, "q", json!(ten))["Successful"]
            .as_array()
            .unwrap()
            .len(),
        10
    );
}

#[test]
fn batch_checks_come_in_spec_order() {
    let mut h = Harness::new();
    let batch = |c: &str, q: Value, e: Value| json!({"Command": c, "QueueName": q, "Entries": e});
    for c in ["SendMessageBatch", "DeleteMessageBatch"] {
        assert_eq!(
            h.err(json!({"Command": c, "Entries": []})),
            "MissingParameter"
        );
        assert_eq!(
            h.err(json!({"Command": c, "QueueName": "q"})),
            "MissingParameter"
        );
        assert_eq!(
            h.err(batch(c, json!("q"), json!({}))),
            "InvalidParameterValue"
        );
        assert_eq!(
            h.err(batch(c, json!(1), json!([]))),
            "InvalidParameterValue"
        );
        assert_eq!(h.err(batch(c, json!("q"), json!([]))), "QueueDoesNotExist");
    }
}

#[test]
fn entry_members_that_are_unknown_or_null_are_ignored_or_absent() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let r = send_batch(
        &mut h,
        "q",
        json!([{"Id": "a", "Body": "x", "Junk": 1, "QueueName": "nope", "GroupId": null}]),
    );
    assert_eq!(r["Successful"], json!([{"Id": "a", "MessageId": "m-1"}]));
    assert_eq!(r["Failed"], json!([]));
}
