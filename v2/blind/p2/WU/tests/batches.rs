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

fn batch_err(command: &str, queue: &str, entries: Value) -> String {
    Harness::new_with_queue(queue)
        .err(json!({"Command": command, "QueueName": queue, "Entries": entries}))
}

impl Harness {
    fn new_with_queue(name: &str) -> Self {
        let mut h = Harness::new();
        create(&mut h, name, json!({}));
        h
    }
}

fn receive(h: &mut Harness, queue: &str, max: u64) -> Vec<Value> {
    let r = h.ok(json!({"Command": "ReceiveMessage", "QueueName": queue, "MaxMessages": max}));
    r["Messages"].as_array().unwrap().clone()
}

#[test]
fn send_batch_numbers_messages_in_order() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
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
    let counts = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}))["Counts"].clone();
    assert_eq!(counts, json!({"Visible": 1, "InFlight": 0, "Delayed": 1}));
}

#[test]
fn failed_entries_do_not_stop_others_or_use_ids() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({"MaxMessageBytes": 3}));
    let r = send_batch(
        &mut h,
        "q",
        json!([
            {"Id": "a", "Body": "abcd"},
            {"Id": "b"},
            {"Id": "c", "Body": 5},
            {"Id": "d", "Body": "ok", "GroupId": "g"},
            {"Id": "e", "Body": "ok", "DelaySeconds": 901},
            {"Id": "f", "Body": "ok", "DelaySeconds": null},
        ]),
    );
    assert_eq!(r["Successful"], json!([{"Id": "f", "MessageId": "m-1"}]));
    let failed: Vec<(&str, &str)> = r["Failed"]
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
        .collect();
    assert_eq!(
        failed,
        [
            ("a", "InvalidParameterValue"),
            ("b", "MissingParameter"),
            ("c", "InvalidParameterValue"),
            ("d", "UnsupportedOperation"),
            ("e", "InvalidParameterValue"),
        ]
    );
}

#[test]
fn entries_see_the_effects_of_earlier_entries() {
    let mut h = Harness::new();
    create(&mut h, "q.fifo", json!({"Fifo": true}));
    let r = send_batch(
        &mut h,
        "q.fifo",
        json!([
            {"Id": "a", "Body": "x", "GroupId": "g", "DeduplicationId": "d"},
            {"Id": "b", "Body": "y", "GroupId": "g", "DeduplicationId": "d"},
            {"Id": "c", "Body": "z", "GroupId": "g", "DeduplicationId": "e"},
            {"Id": "d", "Body": "z", "DeduplicationId": "f"},
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
fn delete_batch_reuses_handles_within_the_batch() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    send_batch(
        &mut h,
        "q",
        json!([{"Id": "a", "Body": "1"}, {"Id": "b", "Body": "2"}]),
    );
    let got = receive(&mut h, "q", 2);
    let handle = |i: usize| got[i]["ReceiptHandle"].clone();
    let r = delete_batch(
        &mut h,
        "q",
        json!([
            {"Id": "a", "ReceiptHandle": handle(0)},
            {"Id": "b", "ReceiptHandle": handle(0)},
            {"Id": "c", "ReceiptHandle": "r-99"},
            {"Id": "d"},
            {"Id": "e", "ReceiptHandle": handle(1)},
        ]),
    );
    assert_eq!(r["Successful"], json!([{"Id": "a"}, {"Id": "e"}]));
    let codes: Vec<&Value> = r["Failed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| &f["error"]["code"])
        .collect();
    assert_eq!(
        codes,
        [
            "ReceiptHandleExpired",
            "ReceiptHandleInvalid",
            "MissingParameter"
        ]
    );
    let ids: Vec<&Value> = r["Failed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| &f["Id"])
        .collect();
    assert_eq!(ids, ["b", "c", "d"]);
    let counts = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}))["Counts"].clone();
    assert_eq!(counts, json!({"Visible": 0, "InFlight": 0, "Delayed": 0}));
}

#[test]
fn message_limit_applies_per_entry() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let entries = |n: usize| -> Value {
        (0..n)
            .map(|i| json!({"Id": format!("e{i}"), "Body": "x"}))
            .collect()
    };
    for _ in 0..9_999 {
        assert_eq!(send_batch(&mut h, "q", entries(10))["Failed"], json!([]));
    }
    assert_eq!(send_batch(&mut h, "q", entries(5))["Failed"], json!([]));
    // 99,995 messages: the first 5 entries fit, the other 5 hit the limit.
    let r = send_batch(&mut h, "q", entries(10));
    assert_eq!(r["Successful"].as_array().unwrap().len(), 5);
    let failed = r["Failed"].as_array().unwrap();
    assert_eq!(failed.len(), 5);
    assert!(failed.iter().all(|f| f["error"]["code"] == "LimitExceeded"));
    assert_eq!(failed[0]["Id"], "e5");
}

#[test]
fn batch_rules_and_their_order() {
    let ok = json!({"Id": "a", "Body": "x"});
    let many = |n: usize| {
        Value::Array(
            (0..n)
                .map(|i| json!({"Id": format!("i{i}"), "Body": "x"}))
                .collect(),
        )
    };
    assert_eq!(
        batch_err("SendMessageBatch", "q", json!([])),
        "EmptyBatchRequest"
    );
    assert_eq!(
        batch_err("DeleteMessageBatch", "q", json!([])),
        "EmptyBatchRequest"
    );
    assert_eq!(
        batch_err("SendMessageBatch", "q", many(11)),
        "TooManyEntriesInBatchRequest"
    );
    let mut h = Harness::new_with_queue("q");
    let r = send_batch(&mut h, "q", many(10));
    assert_eq!(r["Successful"].as_array().unwrap().len(), 10);
    for bad in [
        json!(5),
        json!(null),
        json!({"Body": "x"}),
        json!({"Id": 1}),
        json!({"Id": null}),
        json!({"Id": ""}),
        json!({"Id": "a b"}),
        json!({"Id": "x".repeat(81)}),
    ] {
        assert_eq!(
            batch_err("SendMessageBatch", "q", json!([ok, bad])),
            "InvalidBatchEntryId",
            "{bad}"
        );
    }
    send_batch(&mut h, "q", json!([{"Id": "x".repeat(80), "Body": "x"}]));
    // Rule 3 is checked on every entry before rule 4.
    let entries = json!([ok, ok, {"Id": ""}]);
    assert_eq!(
        batch_err("SendMessageBatch", "q", entries),
        "InvalidBatchEntryId"
    );
    assert_eq!(
        batch_err("SendMessageBatch", "q", json!([ok, {"Id": "b"}, ok])),
        "BatchEntryIdsNotDistinct"
    );
    assert_eq!(
        batch_err(
            "DeleteMessageBatch",
            "q",
            json!([{"Id": "A"}, {"Id": "a"}, {"Id": "A"}])
        ),
        "BatchEntryIdsNotDistinct"
    );
}

#[test]
fn rejected_batch_does_nothing() {
    let mut h = Harness::new_with_queue("q");
    let command = json!({"Command": "SendMessageBatch", "QueueName": "q",
        "Entries": [{"Id": "a", "Body": "x"}, {"Id": "a", "Body": "y"}]});
    assert_eq!(h.err(command), "BatchEntryIdsNotDistinct");
    assert_eq!(receive(&mut h, "q", 10), Vec::<Value>::new());
    // No message id was used up.
    let r = send_batch(&mut h, "q", json!([{"Id": "a", "Body": "x"}]));
    assert_eq!(r["Successful"], json!([{"Id": "a", "MessageId": "m-1"}]));
}

#[test]
fn check_order_before_batch_rules() {
    let mut h = Harness::new();
    for command in ["SendMessageBatch", "DeleteMessageBatch"] {
        assert_eq!(h.err(json!({"Command": command})), "MissingParameter");
        assert_eq!(
            h.err(json!({"Command": command, "QueueName": "q"})),
            "MissingParameter"
        );
        assert_eq!(
            h.err(json!({"Command": command, "Entries": []})),
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
            h.err(json!({"Command": command, "QueueName": "q", "Entries": []})),
            "QueueDoesNotExist"
        );
    }
}
