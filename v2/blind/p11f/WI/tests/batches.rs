mod common;

use common::Harness;
use serde_json::{Value, json};

fn harness() -> Harness {
    let mut h = Harness::new();
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    h
}

fn send_batch(h: &mut Harness, queue: &str, entries: Value) -> Value {
    h.ok(json!({"Command": "SendMessageBatch", "QueueName": queue, "Entries": entries}))
}

fn delete_batch(h: &mut Harness, queue: &str, entries: Value) -> Value {
    h.ok(json!({"Command": "DeleteMessageBatch", "QueueName": queue, "Entries": entries}))
}

fn batch_err(h: &mut Harness, command: &str, entries: Value) -> String {
    h.err(json!({"Command": command, "QueueName": "q", "Entries": entries}))
}

#[test]
fn send_batch_reports_each_entry_in_order() {
    let mut h = harness();
    let r = send_batch(
        &mut h,
        "q",
        json!([{"Id": "a", "Body": "one"}, {"Id": "b"}, {"Id": "c", "Body": "three", "DelaySeconds": 5},
               {"Id": "d", "Body": "x", "GroupId": "g"}, {"Id": "e", "Body": "five", "Extra": 1}]),
    );
    assert_eq!(
        r["Successful"],
        json!([{"Id": "a", "MessageId": "m-1"}, {"Id": "c", "MessageId": "m-2"},
               {"Id": "e", "MessageId": "m-3"}])
    );
    assert_eq!(r["Failed"].as_array().unwrap().len(), 2);
    assert_eq!(r["Failed"][0]["Id"], "b");
    assert_eq!(r["Failed"][0]["error"]["code"], "MissingParameter");
    assert!(r["Failed"][0]["error"]["message"].is_string());
    assert_eq!(r["Failed"][1]["Id"], "d");
    assert_eq!(r["Failed"][1]["error"]["code"], "UnsupportedOperation");
    let counts = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}))["Counts"].clone();
    assert_eq!(counts, json!({"Visible": 2, "InFlight": 0, "Delayed": 1}));
}

#[test]
fn send_batch_with_only_failures_is_still_a_success() {
    let mut h = harness();
    let r = send_batch(
        &mut h,
        "q",
        json!([{"Id": "a", "Body": 5}, {"Id": "b", "Body": ""}]),
    );
    assert_eq!(r["Successful"], json!([]));
    assert_eq!(r["Failed"][0]["error"]["code"], "InvalidParameterValue");
    assert_eq!(r["Failed"][1]["error"]["code"], "InvalidParameterValue");
}

#[test]
fn send_batch_entries_see_earlier_entries_on_a_fifo_queue() {
    let mut h = Harness::new();
    h.ok(json!({"Command": "CreateQueue", "QueueName": "f.fifo", "Attributes": {"Fifo": true}}));
    let r = send_batch(
        &mut h,
        "f.fifo",
        json!([{"Id": "a", "Body": "x", "GroupId": "g", "DeduplicationId": "d"},
               {"Id": "b", "Body": "y", "GroupId": "g", "DeduplicationId": "d"},
               {"Id": "c", "Body": "z", "GroupId": "g", "DeduplicationId": "e"}]),
    );
    assert_eq!(
        r["Successful"],
        json!([{"Id": "a", "MessageId": "m-1", "SequenceNumber": 1},
               {"Id": "b", "MessageId": "m-1", "SequenceNumber": 1},
               {"Id": "c", "MessageId": "m-2", "SequenceNumber": 2}])
    );
    assert_eq!(r["Failed"], json!([]));
}

#[test]
fn send_batch_entries_obey_max_message_bytes_and_the_message_limit() {
    let mut h = Harness::new();
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": {"MaxMessageBytes": 2}}));
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
fn batch_level_checks_fail_the_whole_request() {
    let mut h = harness();
    let eleven: Vec<Value> = (0..11)
        .map(|i| json!({"Id": format!("i{i}"), "Body": "x"}))
        .collect();
    let ten: Vec<Value> = eleven[..10].to_vec();
    for command in ["SendMessageBatch", "DeleteMessageBatch"] {
        assert_eq!(batch_err(&mut h, command, json!([])), "EmptyBatchRequest");
        let code = batch_err(&mut h, command, json!(eleven));
        assert_eq!(code, "TooManyEntriesInBatchRequest");
        for bad in [
            json!([1]),
            json!([null]),
            json!(["a"]),
            json!([{}]),
            json!([{"Id": null}]),
            json!([{"Id": 1}]),
            json!([{"Id": ""}]),
            json!([{"Id": "a b"}]),
            json!([{"Id": "x".repeat(81)}]),
        ] {
            assert_eq!(
                batch_err(&mut h, command, bad.clone()),
                "InvalidBatchEntryId",
                "{bad}"
            );
        }
        let code = batch_err(&mut h, command, json!([{"Id": "a"}, {"Id": "a"}]));
        assert_eq!(code, "BatchEntryIdsNotDistinct");
        let r = h.ok(json!({"Command": command, "QueueName": "q",
            "Entries": [{"Id": "a"}, {"Id": "A"}, {"Id": "x".repeat(80)}]}));
        assert_eq!(r["Failed"].as_array().unwrap().len(), 3);
    }
    assert_eq!(
        send_batch(&mut h, "q", json!(ten))["Successful"]
            .as_array()
            .unwrap()
            .len(),
        10
    );
    let counts = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}))["Counts"].clone();
    assert_eq!(counts["Visible"], 10);
}

#[test]
fn rules_come_in_the_written_order() {
    let mut h = harness();
    let eleven: Vec<Value> = (0..11).map(|_| json!(1)).collect();
    assert_eq!(
        batch_err(&mut h, "SendMessageBatch", json!(eleven)),
        "TooManyEntriesInBatchRequest"
    );
    assert_eq!(
        batch_err(&mut h, "SendMessageBatch", json!([1, 1])),
        "InvalidBatchEntryId"
    );
}

#[test]
fn earlier_checks_decide_before_batch_rules() {
    let mut h = harness();
    for command in ["SendMessageBatch", "DeleteMessageBatch"] {
        assert_eq!(
            h.err(json!({"Command": command, "Entries": []})),
            "MissingParameter"
        );
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
        assert_eq!(
            h.err(json!({"Command": command, "QueueName": "nope", "Entries": []})),
            "QueueDoesNotExist"
        );
    }
}

#[test]
fn delete_batch_deletes_valid_handles_and_reports_the_rest() {
    let mut h = harness();
    send_batch(
        &mut h,
        "q",
        json!([{"Id": "a", "Body": "1"}, {"Id": "b", "Body": "2"}]),
    );
    let r = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 2}));
    let first = r["Messages"][0]["ReceiptHandle"].clone();
    let second = r["Messages"][1]["ReceiptHandle"].clone();
    let r = delete_batch(
        &mut h,
        "q",
        json!([{"Id": "1", "ReceiptHandle": first}, {"Id": "2", "ReceiptHandle": "r-99"},
               {"Id": "3", "ReceiptHandle": first}, {"Id": "4"},
               {"Id": "5", "ReceiptHandle": second}]),
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
            ("3", "ReceiptHandleExpired"),
            ("4", "MissingParameter")
        ]
    );
    let counts = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}))["Counts"].clone();
    assert_eq!(counts, json!({"Visible": 0, "InFlight": 0, "Delayed": 0}));
}
