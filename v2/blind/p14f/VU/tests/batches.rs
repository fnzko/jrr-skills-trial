mod common;

use common::Harness;
use serde_json::{Value, json};

fn harness(attributes: Value) -> Harness {
    let mut h = Harness::new();
    let name = if attributes["Fifo"] == true {
        "q.fifo"
    } else {
        "q"
    };
    h.ok(json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attributes}));
    h
}

fn send_batch(h: &mut Harness, queue: &str, entries: Value) -> Value {
    h.ok(json!({"Command": "SendMessageBatch", "QueueName": queue, "Entries": entries}))
}

#[test]
fn send_batch_numbers_entries_in_order() {
    let mut h = harness(json!({}));
    let r = send_batch(
        &mut h,
        "q",
        json!([{"Id": "a", "Body": "x"}, {"Id": "b", "Body": "y"}]),
    );
    assert_eq!(
        r["Successful"],
        json!([{"Id": "a", "MessageId": "m-1"}, {"Id": "b", "MessageId": "m-2"}])
    );
    assert_eq!(r["Failed"], json!([]));
    let r = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 2}));
    assert_eq!(r["Messages"][0]["Body"], "x");
    assert_eq!(r["Messages"][1]["Body"], "y");
}

#[test]
fn failed_entries_do_not_stop_the_others() {
    let mut h = harness(json!({"MaxMessageBytes": 3}));
    let r = send_batch(
        &mut h,
        "q",
        json!([
            {"Id": "ok1", "Body": "abc"},
            {"Id": "nobody"},
            {"Id": "empty", "Body": ""},
            {"Id": "big", "Body": "abcd"},
            {"Id": "fifoonly", "Body": "a", "GroupId": "g"},
            {"Id": "delay", "Body": "a", "DelaySeconds": 901},
            {"Id": "ok2", "Body": "a", "DelaySeconds": 5, "Extra": true},
        ]),
    );
    assert_eq!(
        r["Successful"],
        json!([{"Id": "ok1", "MessageId": "m-1"}, {"Id": "ok2", "MessageId": "m-2"}])
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
            ("empty", "InvalidParameterValue"),
            ("big", "InvalidParameterValue"),
            ("fifoonly", "UnsupportedOperation"),
            ("delay", "InvalidParameterValue"),
        ]
    );
    assert!(r["Failed"][0]["error"]["message"].is_string());
}

#[test]
fn fifo_batch_sees_deduplication_of_earlier_entries() {
    let mut h = harness(json!({"Fifo": true}));
    let r = send_batch(
        &mut h,
        "q.fifo",
        json!([
            {"Id": "a", "Body": "x", "GroupId": "g", "DeduplicationId": "d"},
            {"Id": "b", "Body": "y", "GroupId": "g", "DeduplicationId": "d"},
            {"Id": "c", "Body": "z", "GroupId": "g", "DeduplicationId": "e"},
            {"Id": "d", "Body": "z", "DelaySeconds": 1, "GroupId": "g", "DeduplicationId": "f"},
            {"Id": "e", "Body": "z", "DeduplicationId": "g"},
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
    assert_eq!(r["Failed"][0]["error"]["code"], "UnsupportedOperation");
    assert_eq!(r["Failed"][1]["error"]["code"], "MissingParameter");
}

#[test]
fn send_batch_respects_the_message_limit() {
    let mut h = harness(json!({}));
    for _ in 0..10_000 {
        let entries: Vec<Value> = (0..10)
            .map(|i| json!({"Id": format!("i{i}"), "Body": "x"}))
            .collect();
        let r = send_batch(&mut h, "q", Value::Array(entries));
        assert_eq!(r["Failed"], json!([]));
    }
    let r = send_batch(&mut h, "q", json!([{"Id": "a", "Body": "x"}]));
    assert_eq!(r["Successful"], json!([]));
    assert_eq!(r["Failed"][0]["error"]["code"], "LimitExceeded");
}

fn receive_handles(h: &mut Harness, n: u64) -> Vec<String> {
    let r = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": n}));
    r["Messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["ReceiptHandle"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn delete_batch_reports_each_entry() {
    let mut h = harness(json!({}));
    send_batch(
        &mut h,
        "q",
        json!([{"Id": "a", "Body": "x"}, {"Id": "b", "Body": "y"}]),
    );
    let handles = receive_handles(&mut h, 2);
    let r = h.ok(
        json!({"Command": "DeleteMessageBatch", "QueueName": "q", "Entries": [
            {"Id": "1", "ReceiptHandle": handles[0]},
            {"Id": "2", "ReceiptHandle": "r-99"},
            {"Id": "3", "ReceiptHandle": handles[0]},
            {"Id": "4"},
            {"Id": "5", "ReceiptHandle": 7},
            {"Id": "6", "ReceiptHandle": handles[1]},
        ]}),
    );
    assert_eq!(r["Successful"], json!([{"Id": "1"}, {"Id": "6"}]));
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
            ("2", "ReceiptHandleInvalid"),
            ("3", "ReceiptHandleExpired"),
            ("4", "MissingParameter"),
            ("5", "InvalidParameterValue"),
        ]
    );
    let counts = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}))["Counts"].clone();
    assert_eq!(counts, json!({"Visible": 0, "InFlight": 0, "Delayed": 0}));
}

#[test]
fn batch_rules_in_order() {
    let mut h = harness(json!({}));
    let ids = |n: usize| -> Vec<Value> {
        (0..n)
            .map(|i| json!({"Id": format!("i{i}"), "Body": "x"}))
            .collect()
    };
    for command in ["SendMessageBatch", "DeleteMessageBatch"] {
        let run = |h: &mut Harness, entries: Value| {
            h.err(json!({"Command": command, "QueueName": "q", "Entries": entries}))
        };
        assert_eq!(run(&mut h, json!([])), "EmptyBatchRequest");
        assert_eq!(
            run(&mut h, Value::Array(ids(11))),
            "TooManyEntriesInBatchRequest"
        );
        // too many wins over a bad id
        let mut eleven = ids(11);
        eleven[0] = json!(5);
        assert_eq!(
            run(&mut h, Value::Array(eleven)),
            "TooManyEntriesInBatchRequest"
        );
        for bad in [
            json!([5]),
            json!([{"Body": "x"}]),
            json!([{"Id": 1}]),
            json!([{"Id": null}]),
            json!([{"Id": ""}]),
            json!([{"Id": "a b"}]),
            json!([{"Id": "x".repeat(81)}]),
            // a bad id wins over duplicates
            json!([{"Id": "a"}, {"Id": "a"}, {"Id": "!"}]),
        ] {
            assert_eq!(run(&mut h, bad), "InvalidBatchEntryId");
        }
        assert_eq!(
            run(&mut h, json!([{"Id": "a"}, {"Id": "a"}])),
            "BatchEntryIdsNotDistinct"
        );
    }
}

#[test]
fn entry_ids_are_case_sensitive_and_may_be_80_characters() {
    let mut h = harness(json!({}));
    let long = "x".repeat(80);
    let r = send_batch(
        &mut h,
        "q",
        json!([{"Id": "a", "Body": "1"}, {"Id": "A", "Body": "2"}, {"Id": long, "Body": "3"}]),
    );
    assert_eq!(r["Successful"].as_array().unwrap().len(), 3);
}

#[test]
fn batch_level_check_order() {
    let mut h = harness(json!({}));
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
        // queue lookup comes before the numbered rules
        assert_eq!(
            h.err(json!({"Command": command, "QueueName": "nope", "Entries": []})),
            "QueueDoesNotExist"
        );
    }
}
