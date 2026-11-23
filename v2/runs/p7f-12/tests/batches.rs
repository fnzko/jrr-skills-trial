mod common;

use common::Harness;
use serde_json::{Value, json};

fn harness() -> Harness {
    let mut h = Harness::new();
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    h
}

fn send_batch(entries: Value) -> Value {
    json!({"Command": "SendMessageBatch", "QueueName": "q", "Entries": entries})
}

fn delete_batch(entries: Value) -> Value {
    json!({"Command": "DeleteMessageBatch", "QueueName": "q", "Entries": entries})
}

fn error_codes(failed: &Value) -> Vec<(&str, &str)> {
    failed
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            assert!(f["error"]["message"].is_string());
            (f["Id"].as_str().unwrap(), f["error"]["code"].as_str().unwrap())
        })
        .collect()
}

#[test]
fn send_batch_assigns_ids_in_order() {
    let mut h = harness();
    let r = h.ok(send_batch(json!([
        {"Id": "a", "Body": "one"}, {"Id": "b", "Body": "two", "DelaySeconds": 5}
    ])));
    assert_eq!(
        r["Successful"],
        json!([{"Id": "a", "MessageId": "m-1"}, {"Id": "b", "MessageId": "m-2"}])
    );
    assert_eq!(r["Failed"], json!([]));
    let c = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(c["Counts"], json!({"Visible": 1, "InFlight": 0, "Delayed": 1}));
}

#[test]
fn send_batch_entry_failures_do_not_stop_the_others() {
    let mut h = harness();
    h.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"MaxMessageBytes": 3}}));
    let r = h.ok(send_batch(json!([
        {"Id": "a", "Body": "toolong"},
        {"Id": "b"},
        {"Id": "c", "Body": 5},
        {"Id": "d", "Body": "ok", "GroupId": "g"},
        {"Id": "e", "Body": "ok", "DelaySeconds": 901},
        {"Id": "f", "Body": "yes", "DelaySeconds": null, "Extra": 1},
    ])));
    assert_eq!(
        error_codes(&r["Failed"]),
        [
            ("a", "InvalidParameterValue"),
            ("b", "MissingParameter"),
            ("c", "InvalidParameterValue"),
            ("d", "UnsupportedOperation"),
            ("e", "InvalidParameterValue"),
        ]
    );
    // a failed entry uses no message id
    assert_eq!(r["Successful"], json!([{"Id": "f", "MessageId": "m-1"}]));
}

#[test]
fn send_batch_on_fifo_sees_earlier_entries() {
    let mut h = Harness::new();
    h.ok(json!({"Command": "CreateQueue", "QueueName": "f.fifo", "Attributes": {"Fifo": true}}));
    let r = h.ok(json!({"Command": "SendMessageBatch", "QueueName": "f.fifo", "Entries": [
        {"Id": "a", "Body": "x", "GroupId": "g", "DeduplicationId": "d"},
        {"Id": "b", "Body": "y", "GroupId": "g", "DeduplicationId": "d"},
        {"Id": "c", "Body": "z", "GroupId": "g", "DeduplicationId": "e"},
        {"Id": "d", "Body": "z", "GroupId": "g"},
        {"Id": "e", "Body": "z", "GroupId": "g", "DelaySeconds": 1, "DeduplicationId": "f"},
    ]}));
    assert_eq!(
        r["Successful"],
        json!([
            {"Id": "a", "MessageId": "m-1", "SequenceNumber": 1},
            {"Id": "b", "MessageId": "m-1", "SequenceNumber": 1},
            {"Id": "c", "MessageId": "m-2", "SequenceNumber": 2},
        ])
    );
    assert_eq!(
        error_codes(&r["Failed"]),
        [("d", "MissingParameter"), ("e", "UnsupportedOperation")]
    );
}

#[test]
fn send_batch_sees_the_message_limit() {
    let mut h = harness();
    for _ in 0..100_000 / 10 - 1 {
        let entries: Vec<_> = (0..10).map(|i| json!({"Id": format!("i{i}"), "Body": "x"})).collect();
        let r = h.ok(send_batch(json!(entries)));
        assert_eq!(r["Failed"], json!([]));
    }
    let entries: Vec<_> = (0..10).map(|i| json!({"Id": format!("i{i}"), "Body": "x"})).collect();
    let r = h.ok(send_batch(json!(entries)));
    assert_eq!(r["Failed"], json!([]));
    let r = h.ok(send_batch(json!([{"Id": "a", "Body": "x"}])));
    assert_eq!(error_codes(&r["Failed"]), [("a", "LimitExceeded")]);
}

#[test]
fn batch_shape_errors_in_rule_order() {
    let mut h = harness();
    let eleven: Vec<_> = (0..11).map(|i| json!({"Id": format!("i{i}"), "Body": "x"})).collect();
    for make in [send_batch as fn(Value) -> Value, delete_batch] {
        assert_eq!(h.err(make(json!([]))), "EmptyBatchRequest");
        assert_eq!(h.err(make(json!(eleven.clone()))), "TooManyEntriesInBatchRequest");
        // too many wins over a bad entry
        let mut bad = eleven.clone();
        bad[0] = json!(1);
        assert_eq!(h.err(make(json!(bad))), "TooManyEntriesInBatchRequest");
        for bad in [
            json!([1]),
            json!([{"Body": "x"}]),
            json!([{"Id": null}]),
            json!([{"Id": 1}]),
            json!([{"Id": ""}]),
            json!([{"Id": "a b"}]),
            json!([{"Id": "x".repeat(81)}]),
            // an invalid id wins over duplicate ids
            json!([{"Id": "a"}, {"Id": "a"}, {"Id": "!"}]),
        ] {
            assert_eq!(h.err(make(bad)), "InvalidBatchEntryId");
        }
        assert_eq!(
            h.err(make(json!([{"Id": "a"}, {"Id": "b"}, {"Id": "a"}]))),
            "BatchEntryIdsNotDistinct"
        );
        // 80 characters is fine
        assert_eq!(h.run(make(json!([{"Id": "x".repeat(80)}])))["ok"], true);
    }
}

#[test]
fn batch_check_order() {
    let mut h = harness();
    for command in ["SendMessageBatch", "DeleteMessageBatch"] {
        assert_eq!(h.err(json!({"Command": command})), "MissingParameter");
        assert_eq!(
            h.err(json!({"Command": command, "QueueName": "q"})),
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
        // the queue lookup comes before the numbered rules
        assert_eq!(
            h.err(json!({"Command": command, "QueueName": "nope", "Entries": []})),
            "QueueDoesNotExist"
        );
    }
}

#[test]
fn a_rejected_batch_does_nothing() {
    let mut h = harness();
    h.err(send_batch(json!([{"Id": "a", "Body": "x"}, {"Id": "a", "Body": "y"}])));
    let r = h.ok(send_batch(json!([{"Id": "a", "Body": "x"}])));
    assert_eq!(r["Successful"], json!([{"Id": "a", "MessageId": "m-1"}]));
}

#[test]
fn delete_batch_handles_each_entry() {
    let mut h = harness();
    h.ok(send_batch(json!([{"Id": "a", "Body": "1"}, {"Id": "b", "Body": "2"}])));
    let r = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 2}));
    let handles: Vec<&str> = r["Messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["ReceiptHandle"].as_str().unwrap())
        .collect();
    assert_eq!(handles, ["r-1", "r-2"]);
    let r = h.ok(delete_batch(json!([
        {"Id": "1", "ReceiptHandle": "r-1"},
        {"Id": "2", "ReceiptHandle": "r-1"},
        {"Id": "3", "ReceiptHandle": "r-99"},
        {"Id": "4"},
        {"Id": "5", "ReceiptHandle": 7},
        {"Id": "6", "ReceiptHandle": "r-2"},
    ])));
    assert_eq!(r["Successful"], json!([{"Id": "1"}, {"Id": "6"}]));
    assert_eq!(
        error_codes(&r["Failed"]),
        [
            ("2", "ReceiptHandleExpired"),
            ("3", "ReceiptHandleInvalid"),
            ("4", "MissingParameter"),
            ("5", "InvalidParameterValue"),
        ]
    );
    let c = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(c["Counts"], json!({"Visible": 0, "InFlight": 0, "Delayed": 0}));
}

#[test]
fn delete_batch_with_only_failures_is_a_success() {
    let mut h = harness();
    let r = h.ok(delete_batch(json!([{"Id": "a", "ReceiptHandle": "r-1"}])));
    assert_eq!(r["Successful"], json!([]));
    assert_eq!(error_codes(&r["Failed"]), [("a", "ReceiptHandleInvalid")]);
}
