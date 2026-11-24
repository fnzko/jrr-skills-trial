mod common;

use common::Harness;
use serde_json::{Value, json};

fn harness() -> Harness {
    let mut h = Harness::new();
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    h
}

fn send_batch(h: &mut Harness, entries: Value) -> Value {
    h.run(json!({"Command": "SendMessageBatch", "QueueName": "q", "Entries": entries}))
}

fn delete_batch(h: &mut Harness, entries: Value) -> Value {
    h.run(json!({"Command": "DeleteMessageBatch", "QueueName": "q", "Entries": entries}))
}

fn code(response: &Value) -> &str {
    response["error"]["code"]
        .as_str()
        .expect("a failure response")
}

#[test]
fn send_batch_reports_each_entry_in_order() {
    let mut h = harness();
    let r = send_batch(
        &mut h,
        json!([{"Id": "a", "Body": "x"}, {"Id": "b", "Body": "y"}]),
    );
    assert_eq!(
        r,
        json!({"ok": true, "Failed": [], "Successful": [
            {"Id": "a", "MessageId": "m-1"}, {"Id": "b", "MessageId": "m-2"}]})
    );
}

#[test]
fn failed_entries_do_not_stop_the_others_or_use_message_ids() {
    let mut h = harness();
    let r = send_batch(
        &mut h,
        json!([
            {"Id": "a", "Body": "x"},
            {"Id": "nobody"},
            {"Id": "badtype", "Body": 5},
            {"Id": "grp", "Body": "x", "GroupId": "g"},
            {"Id": "delay", "Body": "x", "DelaySeconds": 901},
            {"Id": "z", "Body": "z", "QueueName": "ignored", "Other": 1},
        ]),
    );
    assert_eq!(r["ok"], true);
    assert_eq!(
        r["Successful"],
        json!([{"Id": "a", "MessageId": "m-1"}, {"Id": "z", "MessageId": "m-2"}])
    );
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
            ("nobody", "MissingParameter"),
            ("badtype", "InvalidParameterValue"),
            ("grp", "UnsupportedOperation"),
            ("delay", "InvalidParameterValue"),
        ]
    );
    assert!(r["Failed"][0]["error"]["message"].is_string());
}

#[test]
fn null_entry_members_are_absent() {
    let mut h = harness();
    let r = send_batch(
        &mut h,
        json!([{"Id": "a", "Body": "x", "GroupId": null}, {"Id": "b", "Body": null}]),
    );
    assert_eq!(r["Successful"], json!([{"Id": "a", "MessageId": "m-1"}]));
    assert_eq!(r["Failed"][0]["error"]["code"], "MissingParameter");
}

#[test]
fn fifo_batch_deduplicates_within_itself() {
    let mut h = Harness::new();
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q.fifo",
        "Attributes": {"Fifo": true, "ContentBasedDeduplication": true}}));
    let r = h.ok(
        json!({"Command": "SendMessageBatch", "QueueName": "q.fifo", "Entries": [
            {"Id": "a", "Body": "same", "GroupId": "g"},
            {"Id": "b", "Body": "same", "GroupId": "g"},
            {"Id": "c", "Body": "other", "GroupId": "g"},
        ]}),
    );
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
fn batch_respects_max_message_bytes() {
    let mut h = harness();
    h.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"MaxMessageBytes": 2}}));
    let r = send_batch(
        &mut h,
        json!([{"Id": "a", "Body": "ab"}, {"Id": "b", "Body": "abc"}]),
    );
    assert_eq!(r["Successful"], json!([{"Id": "a", "MessageId": "m-1"}]));
    assert_eq!(r["Failed"][0]["Id"], "b");
    assert_eq!(r["Failed"][0]["error"]["code"], "InvalidParameterValue");
}

#[test]
fn batch_rules_fail_the_whole_request_and_do_nothing() {
    let mut h = harness();
    let eleven: Vec<Value> = (0..11)
        .map(|i| json!({"Id": format!("e{i}"), "Body": "x"}))
        .collect();
    let ten: Vec<Value> = eleven[..10].to_vec();
    assert_eq!(code(&send_batch(&mut h, json!([]))), "EmptyBatchRequest");
    assert_eq!(
        code(&send_batch(&mut h, json!(eleven))),
        "TooManyEntriesInBatchRequest"
    );
    for bad in [
        json!([{"Id": "a", "Body": "x"}, 5]),
        json!([{"Id": "a", "Body": "x"}, {"Body": "x"}]),
        json!([{"Id": 1, "Body": "x"}]),
        json!([{"Id": null, "Body": "x"}]),
        json!([{"Id": "", "Body": "x"}]),
        json!([{"Id": "a b", "Body": "x"}]),
        json!([{"Id": "a".repeat(81), "Body": "x"}]),
        json!(["a"]),
    ] {
        assert_eq!(
            code(&send_batch(&mut h, bad.clone())),
            "InvalidBatchEntryId",
            "{bad}"
        );
    }
    assert_eq!(
        code(&send_batch(
            &mut h,
            json!([{"Id": "a", "Body": "x"}, {"Id": "a", "Body": "y"}])
        )),
        "BatchEntryIdsNotDistinct"
    );
    let r = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(r["Counts"]["Visible"], 0);
    // Exactly ten entries and an 80 character Id are fine.
    assert_eq!(
        send_batch(&mut h, json!(ten))["Successful"]
            .as_array()
            .map(Vec::len),
        Some(10)
    );
    let long_id = "a".repeat(80);
    assert_eq!(
        send_batch(&mut h, json!([{"Id": long_id, "Body": "x"}]))["ok"],
        true
    );
}

#[test]
fn batch_checks_parameters_in_the_usual_order() {
    let mut h = harness();
    let cmd = "SendMessageBatch";
    assert_eq!(
        h.err(json!({"Command": cmd, "QueueName": "q"})),
        "MissingParameter"
    );
    assert_eq!(
        h.err(json!({"Command": cmd, "Entries": []})),
        "MissingParameter"
    );
    assert_eq!(
        h.err(json!({"Command": cmd, "QueueName": "q", "Entries": {}})),
        "InvalidParameterValue"
    );
    assert_eq!(
        h.err(json!({"Command": cmd, "QueueName": 1, "Entries": []})),
        "InvalidParameterValue"
    );
    // The lookup comes before the batch rules.
    assert_eq!(
        h.err(json!({"Command": cmd, "QueueName": "nope", "Entries": []})),
        "QueueDoesNotExist"
    );
    assert_eq!(
        h.err(json!({"Command": "DeleteMessageBatch", "QueueName": "nope", "Entries": []})),
        "QueueDoesNotExist"
    );
}

#[test]
fn delete_batch_handles_handles_one_by_one() {
    let mut h = harness();
    for body in ["a", "b"] {
        h.ok(json!({"Command": "SendMessage", "QueueName": "q", "Body": body}));
    }
    h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 2}));
    let r = delete_batch(
        &mut h,
        json!([
            {"Id": "d1", "ReceiptHandle": "r-1"},
            {"Id": "again", "ReceiptHandle": "r-1"},
            {"Id": "never", "ReceiptHandle": "r-9"},
            {"Id": "none"},
            {"Id": "d2", "ReceiptHandle": "r-2"},
        ]),
    );
    assert_eq!(r["Successful"], json!([{"Id": "d1"}, {"Id": "d2"}]));
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
            ("none", "MissingParameter"),
        ]
    );
    let counts = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}))["Counts"].clone();
    assert_eq!(counts, json!({"Visible": 0, "InFlight": 0, "Delayed": 0}));
}

#[test]
fn delete_batch_rules_apply() {
    let mut h = harness();
    assert_eq!(code(&delete_batch(&mut h, json!([]))), "EmptyBatchRequest");
    assert_eq!(
        code(&delete_batch(&mut h, json!([{"ReceiptHandle": "r-1"}]))),
        "InvalidBatchEntryId"
    );
    assert_eq!(
        code(&delete_batch(&mut h, json!([{"Id": "a"}, {"Id": "a"}]))),
        "BatchEntryIdsNotDistinct"
    );
}
