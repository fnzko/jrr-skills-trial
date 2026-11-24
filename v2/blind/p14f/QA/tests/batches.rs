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

fn batch_err(h: &mut Harness, command: &str, entries: Value) -> String {
    h.err(json!({"Command": command, "QueueName": "q", "Entries": entries}))
}

fn visible(h: &mut Harness) -> u64 {
    h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}))["Counts"]["Visible"]
        .as_u64()
        .unwrap()
}

#[test]
fn send_batch_assigns_ids_in_order() {
    let mut h = harness(json!({}));
    let r = send_batch(
        &mut h,
        json!([{"Id": "a", "Body": "one"}, {"Id": "b", "Body": "two", "DelaySeconds": 5}]),
    );
    assert_eq!(
        r["Successful"],
        json!([{"Id": "a", "MessageId": "m-1"}, {"Id": "b", "MessageId": "m-2"}])
    );
    assert_eq!(r["Failed"], json!([]));
    assert_eq!(visible(&mut h), 1);
}

#[test]
fn failed_entries_are_reported_and_do_not_stop_the_others() {
    let mut h = harness(json!({"MaxMessageBytes": 3}));
    let r = send_batch(
        &mut h,
        json!([
            {"Id": "a", "Body": "toolong"},
            {"Id": "b"},
            {"Id": "c", "Body": 5},
            {"Id": "d", "Body": "ok"},
            {"Id": "e", "Body": "ok", "GroupId": "g"},
            {"Id": "f", "Body": "yes", "DelaySeconds": null, "Extra": 1},
        ]),
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
            ("a", "InvalidParameterValue"),
            ("b", "MissingParameter"),
            ("c", "InvalidParameterValue"),
            ("e", "UnsupportedOperation"),
        ]
    );
    assert!(r["Failed"][0]["error"]["message"].is_string());
    // Failed sends use up no message id.
    assert_eq!(
        r["Successful"],
        json!([{"Id": "d", "MessageId": "m-1"}, {"Id": "f", "MessageId": "m-2"}])
    );
}

#[test]
fn all_entries_failing_is_still_a_success() {
    let mut h = harness(json!({}));
    let r = send_batch(&mut h, json!([{"Id": "a"}]));
    assert_eq!(r["Successful"], json!([]));
    assert_eq!(r["Failed"][0]["Id"], "a");
}

#[test]
fn fifo_batch_sees_earlier_entries_for_sequence_numbers_and_deduplication() {
    let mut h = Harness::new();
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q.fifo", "Attributes": {"Fifo": true}}));
    let r = h.ok(
        json!({"Command": "SendMessageBatch", "QueueName": "q.fifo", "Entries": [
            {"Id": "a", "Body": "x", "GroupId": "g", "DeduplicationId": "d1"},
            {"Id": "b", "Body": "y", "GroupId": "g", "DeduplicationId": "d1"},
            {"Id": "c", "Body": "z", "GroupId": "g", "DeduplicationId": "d2"},
            {"Id": "d", "Body": "z", "DeduplicationId": "d3"},
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
    assert_eq!(r["Failed"][0]["Id"], "d");
    assert_eq!(r["Failed"][0]["error"]["code"], "MissingParameter");
}

#[test]
fn batch_shape_errors_in_rule_order() {
    let mut h = harness(json!({}));
    let ok = json!({"Id": "a", "Body": "x"});
    for command in ["SendMessageBatch", "DeleteMessageBatch"] {
        assert_eq!(batch_err(&mut h, command, json!([])), "EmptyBatchRequest");
        assert_eq!(
            batch_err(&mut h, command, json!(vec![ok.clone(); 11])),
            "TooManyEntriesInBatchRequest"
        );
        for bad in [
            json!(5),
            json!({"Body": "x"}),
            json!({"Id": 5}),
            json!({"Id": null}),
            json!({"Id": ""}),
            json!({"Id": "a".repeat(81)}),
            json!({"Id": "a b"}),
        ] {
            assert_eq!(
                batch_err(&mut h, command, json!([ok.clone(), bad.clone()])),
                "InvalidBatchEntryId",
                "{bad}"
            );
        }
        assert_eq!(
            batch_err(&mut h, command, json!([ok.clone(), ok.clone()])),
            "BatchEntryIdsNotDistinct"
        );
        // Rule 3 is checked before rule 4.
        assert_eq!(
            batch_err(&mut h, command, json!([ok.clone(), ok.clone(), {"Id": 1}])),
            "InvalidBatchEntryId"
        );
    }
    assert_eq!(visible(&mut h), 0);
}

#[test]
fn id_of_80_characters_is_accepted() {
    let mut h = harness(json!({}));
    let id = "a-_Z9".repeat(16);
    let r = send_batch(&mut h, json!([{"Id": id, "Body": "x"}]));
    assert_eq!(r["Successful"][0]["Id"], id);
}

#[test]
fn ten_entries_are_accepted() {
    let mut h = harness(json!({}));
    let entries: Vec<Value> = (0..10)
        .map(|i| json!({"Id": format!("e{i}"), "Body": "x"}))
        .collect();
    let r = send_batch(&mut h, json!(entries));
    assert_eq!(r["Successful"].as_array().unwrap().len(), 10);
}

#[test]
fn earlier_checks_win_over_batch_rules() {
    let mut h = harness(json!({}));
    assert_eq!(
        h.err(json!({"Command": "SendMessageBatch", "QueueName": "q"})),
        "MissingParameter"
    );
    assert_eq!(
        h.err(json!({"Command": "DeleteMessageBatch", "Entries": []})),
        "MissingParameter"
    );
    assert_eq!(
        h.err(json!({"Command": "SendMessageBatch", "QueueName": "q", "Entries": "x"})),
        "InvalidParameterValue"
    );
    assert_eq!(
        h.err(json!({"Command": "SendMessageBatch", "QueueName": "nope", "Entries": []})),
        "QueueDoesNotExist"
    );
}

#[test]
fn delete_batch_deletes_and_reports_each_handle() {
    let mut h = harness(json!({}));
    send_batch(
        &mut h,
        json!([{"Id": "a", "Body": "x"}, {"Id": "b", "Body": "y"}]),
    );
    let received = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 2}));
    let handle = |i: usize| received["Messages"][i]["ReceiptHandle"].clone();
    let r = delete_batch(
        &mut h,
        json!([
            {"Id": "1", "ReceiptHandle": handle(0)},
            {"Id": "2", "ReceiptHandle": "r-99"},
            {"Id": "3"},
            {"Id": "4", "ReceiptHandle": handle(0)},
            {"Id": "5", "ReceiptHandle": handle(1)},
        ]),
    );
    assert_eq!(r["Successful"], json!([{"Id": "1"}, {"Id": "5"}]));
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
            ("2", "ReceiptHandleInvalid"),
            ("3", "MissingParameter"),
            ("4", "ReceiptHandleExpired"),
        ]
    );
    let counts = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}))["Counts"].clone();
    assert_eq!(counts, json!({"Visible": 0, "InFlight": 0, "Delayed": 0}));
}

#[test]
fn batch_uses_current_queue_attributes_and_message_ids() {
    let mut h = harness(json!({}));
    let r = send_batch(
        &mut h,
        json!([{"Id": "a", "Body": "x"}, {"Id": "b", "Body": "x"}]),
    );
    assert_eq!(r["Successful"].as_array().unwrap().len(), 2);
    h.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q",
                "Attributes": {"MaxMessageBytes": 1}}));
    let r = send_batch(
        &mut h,
        json!([{"Id": "a", "Body": "xx"}, {"Id": "b", "Body": "x"}]),
    );
    assert_eq!(r["Failed"][0]["Id"], "a");
    assert_eq!(r["Successful"], json!([{"Id": "b", "MessageId": "m-3"}]));
}
