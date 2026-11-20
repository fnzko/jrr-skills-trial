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

fn receive(h: &mut Harness, queue: &str, max: u64) -> Vec<Value> {
    let r = h.ok(json!({"Command": "ReceiveMessage", "QueueName": queue, "MaxMessages": max}));
    r["Messages"].as_array().unwrap().clone()
}

fn error_codes(response: &Value) -> Vec<(&str, &str)> {
    response["Failed"]
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
        .collect()
}

#[test]
fn send_batch_succeeds_in_order_with_consecutive_ids() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
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
    let bodies: Vec<_> = receive(&mut h, "q", 10)
        .iter()
        .map(|m| m["Body"].clone())
        .collect();
    assert_eq!(bodies, [json!("x"), json!("y")]);
}

#[test]
fn failing_entries_are_reported_and_do_not_stop_the_others() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({"MaxMessageBytes": 3}));
    let r = send_batch(
        &mut h,
        "q",
        json!([
            {"Id": "ok1", "Body": "abc"},
            {"Id": "nobody"},
            {"Id": "big", "Body": "abcd"},
            {"Id": "badtype", "Body": 5},
            {"Id": "badgroup", "Body": "x", "GroupId": "g"},
            {"Id": "baddelay", "Body": "x", "DelaySeconds": 901},
            {"Id": "ok2", "Body": "x", "DelaySeconds": null, "Extra": 1},
        ]),
    );
    assert_eq!(
        r["Successful"],
        json!([{"Id": "ok1", "MessageId": "m-1"}, {"Id": "ok2", "MessageId": "m-2"}])
    );
    assert_eq!(
        error_codes(&r),
        [
            ("nobody", "MissingParameter"),
            ("big", "InvalidParameterValue"),
            ("badtype", "InvalidParameterValue"),
            ("badgroup", "UnsupportedOperation"),
            ("baddelay", "InvalidParameterValue"),
        ]
    );
}

#[test]
fn entries_see_the_effects_of_earlier_entries() {
    let mut h = Harness::new();
    create(&mut h, "f.fifo", json!({"Fifo": true}));
    let r = send_batch(
        &mut h,
        "f.fifo",
        json!([
            {"Id": "1", "Body": "a", "GroupId": "g", "DeduplicationId": "d"},
            {"Id": "2", "Body": "b", "GroupId": "g", "DeduplicationId": "d"},
            {"Id": "3", "Body": "c", "GroupId": "g", "DeduplicationId": "e"},
            {"Id": "4", "Body": "c", "GroupId": "g"},
            {"Id": "5", "Body": "c", "DelaySeconds": 1, "GroupId": "g", "DeduplicationId": "z"},
        ]),
    );
    // The duplicate answers with the original's ids and uses up no number.
    assert_eq!(
        r["Successful"],
        json!([
            {"Id": "1", "MessageId": "m-1", "SequenceNumber": 1},
            {"Id": "2", "MessageId": "m-1", "SequenceNumber": 1},
            {"Id": "3", "MessageId": "m-2", "SequenceNumber": 2},
        ])
    );
    assert_eq!(
        error_codes(&r),
        [("4", "MissingParameter"), ("5", "UnsupportedOperation")]
    );
}

#[test]
fn batch_hits_the_message_limit_midway() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    for _ in 0..10_000 {
        let entries: Vec<_> = (0..10)
            .map(|i| json!({"Id": format!("{i}"), "Body": "x"}))
            .collect();
        send_batch(&mut h, "q", Value::Array(entries));
    }
    let r = send_batch(
        &mut h,
        "q",
        json!([{"Id": "a", "Body": "x"}, {"Id": "b", "Body": "x"}]),
    );
    assert_eq!(r["Successful"], json!([]));
    assert_eq!(
        error_codes(&r),
        [("a", "LimitExceeded"), ("b", "LimitExceeded")]
    );
    h.ok(json!({"Command": "PurgeQueue", "QueueName": "q"}));
    let r = send_batch(&mut h, "q", json!([{"Id": "a", "Body": "x"}]));
    assert_eq!(
        r["Successful"],
        json!([{"Id": "a", "MessageId": "m-100001"}])
    );
}

#[test]
fn delete_batch_handles_each_handle() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    create(&mut h, "other", json!({}));
    send_batch(
        &mut h,
        "q",
        json!([{"Id": "a", "Body": "x"}, {"Id": "b", "Body": "y"}]),
    );
    send_batch(&mut h, "other", json!([{"Id": "a", "Body": "z"}]));
    let got = receive(&mut h, "q", 2);
    let foreign = receive(&mut h, "other", 1)[0]["ReceiptHandle"].clone();
    let (h1, h2) = (
        got[0]["ReceiptHandle"].clone(),
        got[1]["ReceiptHandle"].clone(),
    );
    let r = delete_batch(
        &mut h,
        "q",
        json!([
            {"Id": "d1", "ReceiptHandle": h1},
            {"Id": "again", "ReceiptHandle": h1},
            {"Id": "never", "ReceiptHandle": "r-999"},
            {"Id": "missing"},
            {"Id": "num", "ReceiptHandle": 1},
            {"Id": "foreign", "ReceiptHandle": foreign},
            {"Id": "d2", "ReceiptHandle": h2},
        ]),
    );
    assert_eq!(r["Successful"], json!([{"Id": "d1"}, {"Id": "d2"}]));
    assert_eq!(
        error_codes(&r),
        [
            ("again", "ReceiptHandleExpired"),
            ("never", "ReceiptHandleInvalid"),
            ("missing", "MissingParameter"),
            ("num", "InvalidParameterValue"),
            ("foreign", "ReceiptHandleExpired"),
        ]
    );
    let c = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(
        c["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn batch_rules_fail_the_whole_request_and_do_nothing() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let many: Vec<_> = (0..11)
        .map(|i| json!({"Id": format!("i{i}"), "Body": "x"}))
        .collect();
    let cases = [
        (json!([]), "EmptyBatchRequest"),
        (Value::Array(many), "TooManyEntriesInBatchRequest"),
        (json!([{"Id": "a", "Body": "x"}, 5]), "InvalidBatchEntryId"),
        (
            json!([{"Id": "a", "Body": "x"}, null]),
            "InvalidBatchEntryId",
        ),
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
        // Rule 3 comes before rule 4.
        (json!([{"Id": "a"}, {"Id": "a"}, 1]), "InvalidBatchEntryId"),
    ];
    for command in ["SendMessageBatch", "DeleteMessageBatch"] {
        for (entries, code) in &cases {
            assert_eq!(
                h.err(json!({"Command": command, "QueueName": "q", "Entries": entries})),
                *code,
                "{command} {entries}"
            );
        }
    }
    let c = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(c["Counts"]["Visible"], 0);
    // No message id was used up.
    assert_eq!(
        send_batch(&mut h, "q", json!([{"Id": "a", "Body": "x"}]))["Successful"][0]["MessageId"],
        "m-1"
    );
}

#[test]
fn id_boundaries_are_accepted() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let ten: Vec<_> = (0..10)
        .map(|i| json!({"Id": format!("A_-{i}"), "Body": "x"}))
        .collect();
    assert_eq!(
        send_batch(&mut h, "q", Value::Array(ten))["Successful"]
            .as_array()
            .unwrap()
            .len(),
        10
    );
    let long = "z".repeat(80);
    let r = send_batch(&mut h, "q", json!([{"Id": long, "Body": "x"}]));
    assert_eq!(r["Successful"][0]["Id"], json!("z".repeat(80)));
}

#[test]
fn earlier_checks_come_first() {
    let mut h = Harness::new();
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
            h.err(json!({"Command": command, "QueueName": 1, "Entries": []})),
            "InvalidParameterValue"
        );
        for entries in [json!({}), json!("x"), json!(1)] {
            assert_eq!(
                h.err(json!({"Command": command, "QueueName": "q", "Entries": entries})),
                "InvalidParameterValue"
            );
        }
        // The queue lookup (step 4) precedes the batch rules.
        assert_eq!(
            h.err(json!({"Command": command, "QueueName": "q", "Entries": []})),
            "QueueDoesNotExist"
        );
    }
}
