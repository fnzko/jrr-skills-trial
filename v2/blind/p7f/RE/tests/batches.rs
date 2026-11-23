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

fn error_code(response: &Value) -> &str {
    assert_eq!(response["ok"], false, "{response}");
    response["error"]["code"].as_str().unwrap()
}

fn counts(h: &mut Harness, queue: &str) -> Value {
    h.ok(json!({"Command": "GetQueueAttributes", "QueueName": queue}))["Counts"].clone()
}

#[test]
fn send_batch_assigns_ids_in_order() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let r = send_batch(
        &mut h,
        "q",
        json!([{"Id": "a", "Body": "x"}, {"Id": "b", "Body": "y"}]),
    );
    assert_eq!(
        r,
        json!({"ok": true,
               "Successful": [{"Id": "a", "MessageId": "m-1"}, {"Id": "b", "MessageId": "m-2"}],
               "Failed": []})
    );
    let r = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 10}));
    let bodies: Vec<&Value> = r["Messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| &m["Body"])
        .collect();
    assert_eq!(bodies, [&json!("x"), &json!("y")]);
}

#[test]
fn failed_entries_are_reported_and_do_not_stop_others() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let entries = json!([
        {"Id": "ok1", "Body": "x"},
        {"Id": "nobody"},
        {"Id": "badbody", "Body": 5},
        {"Id": "empty", "Body": ""},
        {"Id": "group", "Body": "x", "GroupId": "g"},
        {"Id": "badgroup", "Body": "x", "GroupId": "no good"},
        {"Id": "nulled", "Body": "x", "DelaySeconds": null},
        {"Id": "delayed", "Body": "x", "DelaySeconds": 5},
        {"Id": "baddelay", "Body": "x", "DelaySeconds": 901},
    ]);
    let r = send_batch(&mut h, "q", entries);
    assert_eq!(r["ok"], true);
    assert_eq!(
        r["Successful"],
        json!([
            {"Id": "ok1", "MessageId": "m-1"},
            {"Id": "nulled", "MessageId": "m-2"},
            {"Id": "delayed", "MessageId": "m-3"},
        ])
    );
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
            ("nobody", "MissingParameter"),
            ("badbody", "InvalidParameterValue"),
            ("empty", "InvalidParameterValue"),
            ("group", "UnsupportedOperation"),
            ("badgroup", "InvalidParameterValue"),
            ("baddelay", "InvalidParameterValue"),
        ]
    );
    assert_eq!(
        counts(&mut h, "q"),
        json!({"Visible": 2, "InFlight": 0, "Delayed": 1})
    );
}

#[test]
fn every_entry_failing_is_still_a_success() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let r = send_batch(&mut h, "q", json!([{"Id": "a"}]));
    assert_eq!(r["ok"], true);
    assert_eq!(r["Successful"], json!([]));
    assert_eq!(r["Failed"][0]["error"]["code"], "MissingParameter");
}

#[test]
fn entries_see_the_effects_of_earlier_entries_on_a_fifo_queue() {
    let mut h = Harness::new();
    create(&mut h, "q.fifo", json!({"Fifo": true}));
    let entries = json!([
        {"Id": "a", "Body": "x", "GroupId": "g", "DeduplicationId": "d"},
        {"Id": "dup", "Body": "other", "GroupId": "h", "DeduplicationId": "d"},
        {"Id": "b", "Body": "y", "GroupId": "g", "DeduplicationId": "e"},
        {"Id": "nogroup", "Body": "y", "DeduplicationId": "f"},
        {"Id": "nodedup", "Body": "y", "GroupId": "g"},
        {"Id": "delay", "Body": "y", "GroupId": "g", "DeduplicationId": "z", "DelaySeconds": 1},
    ]);
    let r = send_batch(&mut h, "q.fifo", entries);
    assert_eq!(
        r["Successful"],
        json!([
            {"Id": "a", "MessageId": "m-1", "SequenceNumber": 1},
            {"Id": "dup", "MessageId": "m-1", "SequenceNumber": 1},
            {"Id": "b", "MessageId": "m-2", "SequenceNumber": 2},
        ])
    );
    let codes: Vec<&str> = r["Failed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["error"]["code"].as_str().unwrap())
        .collect();
    assert_eq!(
        codes,
        [
            "MissingParameter",
            "MissingParameter",
            "UnsupportedOperation"
        ]
    );
}

#[test]
fn entries_respect_the_max_message_bytes_of_the_queue() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({"MaxMessageBytes": 2}));
    let r = send_batch(
        &mut h,
        "q",
        json!([{"Id": "a", "Body": "ab"}, {"Id": "b", "Body": "abc"}]),
    );
    assert_eq!(r["Successful"], json!([{"Id": "a", "MessageId": "m-1"}]));
    assert_eq!(r["Failed"][0]["Id"], "b");
    assert_eq!(r["Failed"][0]["error"]["code"], "InvalidParameterValue");
}

#[test]
fn entries_see_the_message_limit() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    // One batch of 10 per command, 10000 commands, fills the queue.
    let entries: Vec<Value> = (0..10)
        .map(|i| json!({"Id": format!("e{i}"), "Body": "x"}))
        .collect();
    let entries = Value::Array(entries);
    for _ in 0..10_000 {
        let r = send_batch(&mut h, "q", entries.clone());
        assert_eq!(r["Failed"], json!([]));
    }
    let r = send_batch(&mut h, "q", json!([{"Id": "a", "Body": "x"}]));
    assert_eq!(r["Failed"][0]["error"]["code"], "LimitExceeded");
    assert_eq!(r["Successful"], json!([]));
}

#[test]
fn batch_rules_come_in_order_and_do_nothing() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let eleven: Vec<Value> = (0..11)
        .map(|i| json!({"Id": format!("e{i}"), "Body": "x"}))
        .collect();
    let ten: Vec<Value> = eleven[..10].to_vec();
    let cases: [(Value, &str); 14] = [
        (json!([]), "EmptyBatchRequest"),
        (Value::Array(eleven.clone()), "TooManyEntriesInBatchRequest"),
        // Too many entries wins over a bad entry.
        (
            Value::Array(eleven.iter().map(|_| json!(5)).collect()),
            "TooManyEntriesInBatchRequest",
        ),
        (json!([5]), "InvalidBatchEntryId"),
        (json!([null]), "InvalidBatchEntryId"),
        (json!([["a"]]), "InvalidBatchEntryId"),
        (json!([{"Body": "x"}]), "InvalidBatchEntryId"),
        (json!([{"Id": null, "Body": "x"}]), "InvalidBatchEntryId"),
        (json!([{"Id": 1, "Body": "x"}]), "InvalidBatchEntryId"),
        (json!([{"Id": "", "Body": "x"}]), "InvalidBatchEntryId"),
        (json!([{"Id": "a b", "Body": "x"}]), "InvalidBatchEntryId"),
        (
            json!([{"Id": "a".repeat(81), "Body": "x"}]),
            "InvalidBatchEntryId",
        ),
        (
            json!([{"Id": "a", "Body": "x"}, {"Id": "a", "Body": "y"}]),
            "BatchEntryIdsNotDistinct",
        ),
        // A bad id comes before duplicate ids.
        (
            json!([{"Id": "a", "Body": "x"}, {"Id": "a", "Body": "y"}, 7]),
            "InvalidBatchEntryId",
        ),
    ];
    for (entries, code) in cases {
        assert_eq!(
            error_code(&send_batch(&mut h, "q", entries.clone())),
            code,
            "{entries}"
        );
        assert_eq!(
            error_code(&delete_batch(&mut h, "q", entries.clone())),
            code,
            "{entries}"
        );
    }
    assert_eq!(
        counts(&mut h, "q"),
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
    // The next message id is still 1: nothing was done.
    let r = send_batch(&mut h, "q", Value::Array(ten));
    assert_eq!(r["Successful"][0]["MessageId"], "m-1");
    assert_eq!(r["Successful"][9]["MessageId"], "m-10");
}

#[test]
fn id_limits_are_inclusive() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let id = "aZ09_-".repeat(13) + "ab";
    assert_eq!(id.len(), 80);
    let r = send_batch(&mut h, "q", json!([{"Id": id, "Body": "x"}]));
    assert_eq!(r["Successful"], json!([{"Id": id, "MessageId": "m-1"}]));
}

#[test]
fn check_order_before_the_batch_rules() {
    let mut h = Harness::new();
    for command in ["SendMessageBatch", "DeleteMessageBatch"] {
        let run = |h: &mut Harness, c: Value| error_code(&h.run(c)).to_owned();
        assert_eq!(
            run(&mut h, json!({"Command": command, "Entries": []})),
            "MissingParameter"
        );
        assert_eq!(
            run(&mut h, json!({"Command": command, "QueueName": "q"})),
            "MissingParameter"
        );
        assert_eq!(
            run(
                &mut h,
                json!({"Command": command, "QueueName": "q", "Entries": null})
            ),
            "MissingParameter"
        );
        for bad in [json!("x"), json!({}), json!(5)] {
            assert_eq!(
                run(
                    &mut h,
                    json!({"Command": command, "QueueName": "q", "Entries": bad})
                ),
                "InvalidParameterValue"
            );
        }
        assert_eq!(
            run(
                &mut h,
                json!({"Command": command, "QueueName": 5, "Entries": []})
            ),
            "InvalidParameterValue"
        );
        // The queue lookup comes before the batch rules.
        assert_eq!(
            run(
                &mut h,
                json!({"Command": command, "QueueName": "q", "Entries": []})
            ),
            "QueueDoesNotExist"
        );
    }
}

#[test]
fn delete_batch_deletes_and_reports_failures() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    create(&mut h, "other", json!({}));
    for body in ["a", "b", "c"] {
        h.ok(json!({"Command": "SendMessage", "QueueName": "q", "Body": body}));
    }
    h.ok(json!({"Command": "SendMessage", "QueueName": "other", "Body": "o"}));
    h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 3}));
    h.ok(json!({"Command": "ReceiveMessage", "QueueName": "other"}));
    let entries = json!([
        {"Id": "one", "ReceiptHandle": "r-1"},
        {"Id": "again", "ReceiptHandle": "r-1"},
        {"Id": "never", "ReceiptHandle": "r-99"},
        {"Id": "odd", "ReceiptHandle": "r-01"},
        {"Id": "foreign", "ReceiptHandle": "r-4"},
        {"Id": "missing"},
        {"Id": "wrongtype", "ReceiptHandle": 3},
        {"Id": "three", "ReceiptHandle": "r-3", "Body": "ignored", "QueueName": "other"},
    ]);
    let r = delete_batch(&mut h, "q", entries);
    assert_eq!(r["ok"], true);
    assert_eq!(r["Successful"], json!([{"Id": "one"}, {"Id": "three"}]));
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
            ("again", "ReceiptHandleExpired"),
            ("never", "ReceiptHandleInvalid"),
            ("odd", "ReceiptHandleInvalid"),
            ("foreign", "ReceiptHandleExpired"),
            ("missing", "MissingParameter"),
            ("wrongtype", "InvalidParameterValue"),
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
fn delete_batch_with_only_failures_is_a_success() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let r = delete_batch(&mut h, "q", json!([{"Id": "a", "ReceiptHandle": "r-1"}]));
    assert_eq!(r["ok"], true);
    assert_eq!(r["Successful"], json!([]));
    assert_eq!(r["Failed"][0]["error"]["code"], "ReceiptHandleInvalid");
}

#[test]
fn queue_name_in_entries_is_ignored() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let r = send_batch(
        &mut h,
        "q",
        json!([{"Id": "a", "Body": "x", "QueueName": "nope"}]),
    );
    assert_eq!(r["Successful"], json!([{"Id": "a", "MessageId": "m-1"}]));
}
