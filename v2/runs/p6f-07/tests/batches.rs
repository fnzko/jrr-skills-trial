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

fn code(response: &Value) -> &str {
    assert_eq!(response["ok"], false, "{response}");
    response["error"]["code"].as_str().unwrap()
}

#[test]
fn send_batch_reports_ids_in_order_and_continues_after_failure() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let r = send_batch(
        &mut h,
        "q",
        json!([
            {"Id": "a", "Body": "one"},
            {"Id": "b"},
            {"Id": "c", "Body": "three", "DelaySeconds": 901},
            {"Id": "d", "Body": "four", "GroupId": "g"},
            {"Id": "e", "Body": "five", "DelaySeconds": 5},
        ]),
    );
    assert_eq!(r["ok"], true);
    // failed sends do not use up message ids
    assert_eq!(
        r["Successful"],
        json!([{"Id": "a", "MessageId": "m-1"}, {"Id": "e", "MessageId": "m-2"}])
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
            ("b", "MissingParameter"),
            ("c", "InvalidParameterValue"),
            ("d", "UnsupportedOperation"),
        ]
    );
    assert!(r["Failed"][0]["error"]["message"].is_string());
    let counts = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}))["Counts"].clone();
    assert_eq!(counts, json!({"Visible": 1, "InFlight": 0, "Delayed": 1}));
}

#[test]
fn send_batch_entries_see_earlier_effects_on_fifo_queue() {
    let mut h = Harness::new();
    create(&mut h, "q.fifo", json!({"Fifo": true}));
    let r = send_batch(
        &mut h,
        "q.fifo",
        json!([
            {"Id": "1", "Body": "x", "GroupId": "g", "DeduplicationId": "d"},
            {"Id": "2", "Body": "y", "GroupId": "g", "DeduplicationId": "d"},
            {"Id": "3", "Body": "z", "GroupId": "g", "DeduplicationId": "e"},
            {"Id": "4", "Body": "z", "GroupId": "g"},
        ]),
    );
    assert_eq!(
        r["Successful"],
        json!([
            {"Id": "1", "MessageId": "m-1", "SequenceNumber": 1},
            {"Id": "2", "MessageId": "m-1", "SequenceNumber": 1},
            {"Id": "3", "MessageId": "m-2", "SequenceNumber": 2},
        ])
    );
    assert_eq!(r["Failed"][0]["Id"], "4");
    assert_eq!(r["Failed"][0]["error"]["code"], "MissingParameter");
}

#[test]
fn send_batch_applies_message_limit_and_size_limit_per_entry() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({"MaxMessageBytes": 3}));
    let r = send_batch(
        &mut h,
        "q",
        json!([{"Id": "a", "Body": "abc"}, {"Id": "b", "Body": "abcd"}, {"Id": "c", "Body": ""}]),
    );
    assert_eq!(r["Successful"], json!([{"Id": "a", "MessageId": "m-1"}]));
    assert_eq!(r["Failed"][0]["error"]["code"], "InvalidParameterValue");
    assert_eq!(r["Failed"][1]["error"]["code"], "InvalidParameterValue");
}

#[test]
fn send_batch_hits_the_queue_limit_midway() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    for _ in 0..(100_000 / 10) - 1 {
        let entries: Vec<Value> = (0..10)
            .map(|i| json!({"Id": format!("i{i}"), "Body": "x"}))
            .collect();
        assert_eq!(send_batch(&mut h, "q", json!(entries))["Failed"], json!([]));
    }
    let entries: Vec<Value> = (0..10)
        .map(|i| json!({"Id": format!("i{i}"), "Body": "x"}))
        .collect();
    // the queue now holds exactly 100000 messages
    assert_eq!(send_batch(&mut h, "q", json!(entries))["Failed"], json!([]));
    let r = send_batch(&mut h, "q", json!([{"Id": "late", "Body": "x"}]));
    assert_eq!(r["Failed"][0]["error"]["code"], "LimitExceeded");
    assert_eq!(r["Successful"], json!([]));
}

#[test]
fn batch_rules_and_their_order() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let eleven: Vec<Value> = (0..11)
        .map(|i| json!({"Id": format!("i{i}"), "Body": "x"}))
        .collect();
    let ten: Vec<Value> = eleven[..10].to_vec();
    assert_eq!(
        code(&send_batch(&mut h, "q", json!([]))),
        "EmptyBatchRequest"
    );
    assert_eq!(
        code(&send_batch(&mut h, "q", json!(eleven))),
        "TooManyEntriesInBatchRequest"
    );
    assert_eq!(send_batch(&mut h, "q", json!(ten))["ok"], true);
    let bad_ids = [
        json!(["text"]),
        json!([{"Body": "x"}]),
        json!([{"Id": null, "Body": "x"}]),
        json!([{"Id": 1, "Body": "x"}]),
        json!([{"Id": "", "Body": "x"}]),
        json!([{"Id": "a b", "Body": "x"}]),
        json!([{"Id": "a".repeat(81), "Body": "x"}]),
    ];
    for entries in bad_ids {
        assert_eq!(
            code(&send_batch(&mut h, "q", entries.clone())),
            "InvalidBatchEntryId",
            "{entries}"
        );
    }
    assert_eq!(
        send_batch(&mut h, "q", json!([{"Id": "a".repeat(80), "Body": "x"}]))["ok"],
        true
    );
    // rule 3 comes before rule 4
    let r = send_batch(&mut h, "q", json!([{"Id": "a"}, {"Id": "a"}, {"Id": "!"}]));
    assert_eq!(code(&r), "InvalidBatchEntryId");
    let r = send_batch(
        &mut h,
        "q",
        json!([{"Id": "a", "Body": "x"}, {"Id": "a", "Body": "x"}]),
    );
    assert_eq!(code(&r), "BatchEntryIdsNotDistinct");
    // ids are case-sensitive
    assert_eq!(
        send_batch(
            &mut h,
            "q",
            json!([{"Id": "a", "Body": "x"}, {"Id": "A", "Body": "x"}])
        )["ok"],
        true
    );
}

#[test]
fn rejected_batch_does_nothing() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let r = send_batch(
        &mut h,
        "q",
        json!([{"Id": "a", "Body": "x"}, {"Id": "a", "Body": "x"}]),
    );
    assert_eq!(code(&r), "BatchEntryIdsNotDistinct");
    let counts = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}))["Counts"].clone();
    assert_eq!(counts, json!({"Visible": 0, "InFlight": 0, "Delayed": 0}));
    let r = send_batch(&mut h, "q", json!([{"Id": "a", "Body": "x"}]));
    assert_eq!(r["Successful"], json!([{"Id": "a", "MessageId": "m-1"}]));
}

#[test]
fn batch_parameter_errors_follow_the_check_order() {
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
        for bad in [json!({}), json!("x"), json!(1)] {
            let c = json!({"Command": command, "QueueName": "q", "Entries": bad});
            assert_eq!(h.err(c), "InvalidParameterValue");
        }
        assert_eq!(
            h.err(json!({"Command": command, "QueueName": 1, "Entries": []})),
            "InvalidParameterValue"
        );
        // lookup comes before the numbered batch rules
        assert_eq!(
            h.err(json!({"Command": command, "QueueName": "q", "Entries": []})),
            "QueueDoesNotExist"
        );
    }
}

#[test]
fn delete_batch_deletes_and_reports_per_entry() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    create(&mut h, "other", json!({}));
    for body in ["a", "b"] {
        h.ok(json!({"Command": "SendMessage", "QueueName": "q", "Body": body}));
    }
    h.ok(json!({"Command": "SendMessage", "QueueName": "other", "Body": "o"}));
    let received = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 2}));
    let handle = |i: usize| received["Messages"][i]["ReceiptHandle"].clone();
    let foreign = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "other"}))["Messages"][0]["ReceiptHandle"].clone();
    assert_eq!(handle(0), "r-1");
    let r = delete_batch(
        &mut h,
        "q",
        json!([
            {"Id": "1", "ReceiptHandle": handle(0)},
            {"Id": "2", "ReceiptHandle": handle(0)},
            {"Id": "3"},
            {"Id": "4", "ReceiptHandle": "r-99"},
            {"Id": "5", "ReceiptHandle": foreign},
            {"Id": "6", "ReceiptHandle": handle(1), "Body": "ignored"},
        ]),
    );
    assert_eq!(r["ok"], true);
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
            ("2", "ReceiptHandleExpired"),
            ("3", "MissingParameter"),
            ("4", "ReceiptHandleInvalid"),
            ("5", "ReceiptHandleExpired"),
        ]
    );
    let counts = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}))["Counts"].clone();
    assert_eq!(counts, json!({"Visible": 0, "InFlight": 0, "Delayed": 0}));
}

#[test]
fn delete_batch_with_all_entries_failing_is_still_a_success() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let r = delete_batch(&mut h, "q", json!([{"Id": "x", "ReceiptHandle": 5}]));
    assert_eq!(r["ok"], true);
    assert_eq!(r["Successful"], json!([]));
    assert_eq!(r["Failed"][0]["error"]["code"], "InvalidParameterValue");
}
