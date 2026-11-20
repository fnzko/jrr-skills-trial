mod common;

use common::Harness;
use serde_json::{Value, json};

fn harness(attributes: Value) -> Harness {
    let mut h = Harness::new();
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": attributes}));
    h
}

fn send_batch(h: &mut Harness, entries: Value) -> Value {
    h.ok(json!({"Command": "SendMessageBatch", "QueueName": "q", "Entries": entries}))
}

fn delete_batch(h: &mut Harness, entries: Value) -> Value {
    h.ok(json!({"Command": "DeleteMessageBatch", "QueueName": "q", "Entries": entries}))
}

fn receive(h: &mut Harness, max: u64) -> Vec<Value> {
    let r = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": max}));
    r["Messages"].as_array().unwrap().clone()
}

fn counts(h: &mut Harness) -> Value {
    h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}))["Counts"].clone()
}

#[test]
fn send_batch_numbers_entries_in_order() {
    let mut h = harness(json!({}));
    let r = send_batch(
        &mut h,
        json!([{"Id": "a", "Body": "one"}, {"Id": "b", "Body": "two", "DelaySeconds": 5}]),
    );
    assert_eq!(
        r,
        json!({"ok": true, "Failed": [], "Successful": [
            {"Id": "a", "MessageId": "m-1"}, {"Id": "b", "MessageId": "m-2"}]})
    );
    assert_eq!(
        counts(&mut h),
        json!({"Visible": 1, "InFlight": 0, "Delayed": 1})
    );
    let bodies: Vec<_> = receive(&mut h, 10)
        .iter()
        .map(|m| m["Body"].clone())
        .collect();
    assert_eq!(bodies, [json!("one")]);
}

#[test]
fn failed_entries_are_reported_and_do_not_stop_the_others() {
    let mut h = harness(json!({"MaxMessageBytes": 3}));
    let r = send_batch(
        &mut h,
        json!([
            {"Id": "a", "Body": "ok"},
            {"Id": "b"},
            {"Id": "c", "Body": 5},
            {"Id": "d", "Body": "toolong"},
            {"Id": "e", "Body": "x", "GroupId": "g"},
            {"Id": "f", "Body": "ok2", "ignored": true, "DelaySeconds": null},
        ]),
    );
    assert_eq!(r["ok"], true);
    assert_eq!(
        r["Successful"],
        json!([{"Id": "a", "MessageId": "m-1"}, {"Id": "f", "MessageId": "m-2"}])
    );
    let failed = r["Failed"].as_array().unwrap();
    let summary: Vec<_> = failed
        .iter()
        .map(|f| {
            (
                f["Id"].as_str().unwrap(),
                f["error"]["code"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        summary,
        [
            ("b", "MissingParameter"),
            ("c", "InvalidParameterValue"),
            ("d", "InvalidParameterValue"),
            ("e", "UnsupportedOperation"),
        ]
    );
    assert!(failed.iter().all(|f| f["error"]["message"].is_string()));
    assert_eq!(counts(&mut h)["Visible"], 2);
}

#[test]
fn every_entry_failing_is_still_a_success() {
    let mut h = harness(json!({}));
    let r = send_batch(&mut h, json!([{"Id": "a"}]));
    assert_eq!(r["Successful"], json!([]));
    assert_eq!(r["Failed"].as_array().unwrap().len(), 1);
}

#[test]
fn entries_see_the_effects_of_earlier_entries() {
    let mut h = Harness::new();
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q.fifo",
        "Attributes": {"Fifo": true, "ContentBasedDeduplication": true}}));
    let r = h.ok(
        json!({"Command": "SendMessageBatch", "QueueName": "q.fifo", "Entries": [
            {"Id": "1", "Body": "x", "GroupId": "g"},
            {"Id": "2", "Body": "y", "GroupId": "g"},
            {"Id": "3", "Body": "x", "GroupId": "g"},
        ]}),
    );
    assert_eq!(
        r["Successful"],
        json!([
            {"Id": "1", "MessageId": "m-1", "SequenceNumber": 1},
            {"Id": "2", "MessageId": "m-2", "SequenceNumber": 2},
            // A duplicate succeeds with the original ids and uses no number.
            {"Id": "3", "MessageId": "m-1", "SequenceNumber": 1},
        ])
    );
    let r = h.ok(
        json!({"Command": "SendMessageBatch", "QueueName": "q.fifo", "Entries": [
        {"Id": "4", "Body": "z", "GroupId": "g"}]}),
    );
    assert_eq!(r["Successful"][0]["MessageId"], "m-3");
}

#[test]
fn the_message_limit_applies_entry_by_entry() {
    let mut h = harness(json!({}));
    for _ in 0..10_000 {
        let entries: Vec<Value> = (0..10)
            .map(|i| json!({"Id": format!("e{i}"), "Body": "b"}))
            .collect();
        let r = send_batch(&mut h, Value::Array(entries));
        assert_eq!(r["Failed"], json!([]));
    }
    let r = send_batch(&mut h, json!([{"Id": "a", "Body": "b"}]));
    assert_eq!(r["Failed"][0]["error"]["code"], "LimitExceeded");
}

#[test]
fn batch_rules_and_their_order() {
    let mut h = harness(json!({}));
    let run = |h: &mut Harness, entries: Value| {
        h.err(json!({"Command": "SendMessageBatch", "QueueName": "q", "Entries": entries}))
    };
    assert_eq!(run(&mut h, json!([])), "EmptyBatchRequest");
    let eleven: Vec<Value> = (0..11)
        .map(|i| json!({"Id": format!("e{i}"), "Body": "b"}))
        .collect();
    assert_eq!(
        run(&mut h, Value::Array(eleven)),
        "TooManyEntriesInBatchRequest"
    );
    // Rule 2 comes before rules 3 and 4.
    assert_eq!(
        run(&mut h, json!([1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11])),
        "TooManyEntriesInBatchRequest"
    );
    let ten: Vec<Value> = (0..10)
        .map(|i| json!({"Id": format!("e{i}"), "Body": "b"}))
        .collect();
    send_batch(&mut h, Value::Array(ten));

    for bad in [
        json!([1]),
        json!([null]),
        json!(["a"]),
        json!([[]]),
        json!([{"Body": "b"}]),
        json!([{"Id": null, "Body": "b"}]),
        json!([{"Id": 1, "Body": "b"}]),
        json!([{"Id": "", "Body": "b"}]),
        json!([{"Id": "a b", "Body": "b"}]),
        json!([{"Id": "x".repeat(81), "Body": "b"}]),
        // Rule 3 beats rule 4 and beats entry failures.
        json!([{"Id": "a"}, {"Id": "a"}, {"Body": "b"}]),
    ] {
        assert_eq!(run(&mut h, bad.clone()), "InvalidBatchEntryId", "{bad}");
    }
    send_batch(
        &mut h,
        json!([{"Id": "x".repeat(80), "Body": "b"}, {"Id": "A_-9", "Body": "b"}]),
    );
    assert_eq!(
        run(
            &mut h,
            json!([{"Id": "a", "Body": "b"}, {"Id": "a", "Body": "c"}])
        ),
        "BatchEntryIdsNotDistinct"
    );
    // Ids are case-sensitive.
    send_batch(
        &mut h,
        json!([{"Id": "a", "Body": "b"}, {"Id": "A", "Body": "c"}]),
    );
    // A failed request does nothing.
    let before = counts(&mut h);
    run(
        &mut h,
        json!([{"Id": "a", "Body": "b"}, {"Id": "a", "Body": "c"}]),
    );
    assert_eq!(counts(&mut h), before);
}

#[test]
fn batch_check_order_before_the_numbered_rules() {
    let mut h = harness(json!({}));
    let c = |name: Value, entries: Value| {
        let mut command = json!({"Command": "SendMessageBatch"});
        if !name.is_null() {
            command["QueueName"] = name;
        }
        if !entries.is_null() {
            command["Entries"] = entries;
        }
        command
    };
    assert_eq!(h.err(c(Value::Null, json!([]))), "MissingParameter");
    assert_eq!(h.err(c(json!("q"), Value::Null)), "MissingParameter");
    assert_eq!(h.err(c(json!(1), Value::Null)), "MissingParameter");
    assert_eq!(h.err(c(json!("q"), json!({}))), "InvalidParameterValue");
    assert_eq!(h.err(c(json!("q"), json!("x"))), "InvalidParameterValue");
    assert_eq!(h.err(c(json!(1), json!([]))), "InvalidParameterValue");
    // The lookup comes before the rules on Entries.
    assert_eq!(h.err(c(json!("nope"), json!([]))), "QueueDoesNotExist");
    assert_eq!(h.err(c(json!("q"), json!([]))), "EmptyBatchRequest");
    let mut d = c(json!("nope"), json!([]));
    d["Command"] = json!("DeleteMessageBatch");
    assert_eq!(h.err(d), "QueueDoesNotExist");
}

#[test]
fn delete_batch_handles_each_entry_like_delete_message() {
    let mut h = harness(json!({}));
    send_batch(
        &mut h,
        json!([{"Id": "a", "Body": "1"}, {"Id": "b", "Body": "2"}, {"Id": "c", "Body": "3"}]),
    );
    let got = receive(&mut h, 3);
    let handles: Vec<&str> = got
        .iter()
        .map(|m| m["ReceiptHandle"].as_str().unwrap())
        .collect();
    assert_eq!(handles, ["r-1", "r-2", "r-3"]);
    let r = delete_batch(
        &mut h,
        json!([
            {"Id": "a", "ReceiptHandle": "r-1"},
            {"Id": "b", "ReceiptHandle": "r-99"},
            {"Id": "c"},
            {"Id": "d", "ReceiptHandle": 7},
            // Handles already used by an earlier entry are gone.
            {"Id": "e", "ReceiptHandle": "r-1"},
            {"Id": "f", "ReceiptHandle": "r-3"},
        ]),
    );
    assert_eq!(r["Successful"], json!([{"Id": "a"}, {"Id": "f"}]));
    let summary: Vec<_> = r["Failed"]
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
        summary,
        [
            ("b", "ReceiptHandleInvalid"),
            ("c", "MissingParameter"),
            ("d", "InvalidParameterValue"),
            ("e", "ReceiptHandleExpired"),
        ]
    );
    assert_eq!(
        counts(&mut h),
        json!({"Visible": 0, "InFlight": 1, "Delayed": 0})
    );
}

#[test]
fn delete_batch_rejects_a_handle_of_another_queue() {
    let mut h = harness(json!({}));
    h.ok(json!({"Command": "CreateQueue", "QueueName": "other"}));
    h.ok(json!({"Command": "SendMessage", "QueueName": "other", "Body": "x"}));
    h.ok(json!({"Command": "ReceiveMessage", "QueueName": "other"}));
    let r = delete_batch(&mut h, json!([{"Id": "a", "ReceiptHandle": "r-1"}]));
    assert_eq!(r["Failed"][0]["error"]["code"], "ReceiptHandleExpired");
}
