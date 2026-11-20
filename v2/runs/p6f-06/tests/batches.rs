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
fn send_batch_assigns_consecutive_ids_in_entry_order() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    h.ok(json!({"Command": "SendMessage", "QueueName": "q", "Body": "first"}));
    let r = send_batch(
        &mut h,
        "q",
        json!([
            {"Id": "a", "Body": "x"},
            {"Id": "b", "Body": "y", "DelaySeconds": 5},
        ]),
    );
    assert_eq!(r["ok"], true);
    assert_eq!(r["Failed"], json!([]));
    assert_eq!(
        r["Successful"],
        json!([{"Id": "a", "MessageId": "m-2"}, {"Id": "b", "MessageId": "m-3"}])
    );
    let counts = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}))["Counts"].clone();
    assert_eq!(counts, json!({"Visible": 2, "InFlight": 0, "Delayed": 1}));
}

#[test]
fn failed_entries_do_not_stop_the_others_or_use_ids() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({"MaxMessageBytes": 3}));
    let r = send_batch(
        &mut h,
        "q",
        json!([
            {"Id": "a", "Body": "long body"},
            {"Id": "b"},
            {"Id": "c", "Body": "ok", "GroupId": "g"},
            {"Id": "d", "Body": "ok", "DelaySeconds": 901},
            {"Id": "e", "Body": "ok", "DelaySeconds": null, "Extra": 1},
        ]),
    );
    assert_eq!(r["ok"], true);
    assert_eq!(r["Successful"], json!([{"Id": "e", "MessageId": "m-1"}]));
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
            ("c", "UnsupportedOperation"),
            ("d", "InvalidParameterValue"),
        ]
    );
    assert!(r["Failed"][0]["error"]["message"].is_string());
}

#[test]
fn fifo_entries_see_deduplication_and_sequence_numbers_of_earlier_entries() {
    let mut h = Harness::new();
    create(&mut h, "q.fifo", json!({"Fifo": true}));
    let r = send_batch(
        &mut h,
        "q.fifo",
        json!([
            {"Id": "a", "Body": "x", "GroupId": "g", "DeduplicationId": "d"},
            {"Id": "b", "Body": "y", "GroupId": "g", "DeduplicationId": "d"},
            {"Id": "c", "Body": "z", "GroupId": "g", "DeduplicationId": "e"},
            {"Id": "d", "Body": "z", "GroupId": "g"},
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
fn batch_entries_enforce_the_queue_message_limit() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    for _ in 0..10_000 {
        let entries: Vec<Value> = (0..10)
            .map(|i| json!({"Id": format!("i{i}"), "Body": "x"}))
            .collect();
        assert_eq!(send_batch(&mut h, "q", json!(entries))["Failed"], json!([]));
    }
    let r = send_batch(
        &mut h,
        "q",
        json!([{"Id": "a", "Body": "x"}, {"Id": "b", "Body": "x"}]),
    );
    assert_eq!(r["Successful"], json!([]));
    assert_eq!(r["Failed"][0]["error"]["code"], "LimitExceeded");
    assert_eq!(r["Failed"][1]["error"]["code"], "LimitExceeded");
}

#[test]
fn batch_rules_are_checked_in_order_after_the_queue_lookup() {
    let mut h = Harness::new();
    let eleven: Vec<Value> = (0..11)
        .map(|i| json!({"Id": format!("i{i}"), "Body": "x"}))
        .collect();
    for command in ["SendMessageBatch", "DeleteMessageBatch"] {
        let run = |h: &mut Harness, queue: &str, entries: Value| {
            h.run(json!({"Command": command, "QueueName": queue, "Entries": entries}))
        };
        assert_eq!(
            h.err(json!({"Command": command, "QueueName": "q"})),
            "MissingParameter"
        );
        assert_eq!(
            h.err(json!({"Command": command, "Entries": []})),
            "MissingParameter"
        );
        assert_eq!(code(&run(&mut h, "q", json!("x"))), "InvalidParameterValue");
        assert_eq!(code(&run(&mut h, "nope", json!([]))), "QueueDoesNotExist");
        create(&mut h, "q", json!({}));
        assert_eq!(code(&run(&mut h, "q", json!([]))), "EmptyBatchRequest");
        assert_eq!(
            code(&run(&mut h, "q", json!(eleven))),
            "TooManyEntriesInBatchRequest"
        );
        // Too many entries is checked before the entries themselves.
        let mut bad = eleven.clone();
        bad[0] = json!(1);
        assert_eq!(
            code(&run(&mut h, "q", json!(bad))),
            "TooManyEntriesInBatchRequest"
        );
        for entry in [
            json!(1),
            json!({}),
            json!({"Id": 1}),
            json!({"Id": null}),
            json!({"Id": ""}),
            json!({"Id": "a b"}),
            json!({"Id": "x".repeat(81)}),
        ] {
            let entries = json!([{"Id": "ok", "Body": "x", "ReceiptHandle": "r-1"}, entry]);
            assert_eq!(code(&run(&mut h, "q", entries)), "InvalidBatchEntryId");
        }
        // An invalid Id wins over duplicate Ids.
        let entries = json!([{"Id": "a"}, {"Id": "a"}, {"Id": 3}]);
        assert_eq!(code(&run(&mut h, "q", entries)), "InvalidBatchEntryId");
        let entries = json!([{"Id": "a"}, {"Id": "b"}, {"Id": "a"}]);
        assert_eq!(code(&run(&mut h, "q", entries)), "BatchEntryIdsNotDistinct");
        let ok = json!([{"Id": "x".repeat(80)}, {"Id": "A"}, {"Id": "a"}, {"Id": "_-9"}]);
        assert_eq!(run(&mut h, "q", ok)["ok"], true);
        h.ok(json!({"Command": "DeleteQueue", "QueueName": "q"}));
    }
}

#[test]
fn rejected_batch_does_nothing() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let r = send_batch(
        &mut h,
        "q",
        json!([{"Id": "a", "Body": "x"}, {"Id": "a", "Body": "y"}]),
    );
    assert_eq!(code(&r), "BatchEntryIdsNotDistinct");
    let counts = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}))["Counts"].clone();
    assert_eq!(counts["Visible"], 0);
    let r = h.ok(json!({"Command": "SendMessage", "QueueName": "q", "Body": "x"}));
    assert_eq!(r["MessageId"], "m-1");
}

#[test]
fn delete_batch_removes_messages_and_reports_each_entry() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    create(&mut h, "other", json!({}));
    for body in ["a", "b", "c"] {
        h.ok(json!({"Command": "SendMessage", "QueueName": "q", "Body": body}));
    }
    h.ok(json!({"Command": "SendMessage", "QueueName": "other", "Body": "o"}));
    h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 3}));
    h.ok(json!({"Command": "ReceiveMessage", "QueueName": "other"}));
    let r = delete_batch(
        &mut h,
        "q",
        json!([
            {"Id": "1", "ReceiptHandle": "r-1"},
            {"Id": "2", "ReceiptHandle": "r-1"},
            {"Id": "3", "ReceiptHandle": "r-4"},
            {"Id": "4", "ReceiptHandle": "r-99"},
            {"Id": "5"},
            {"Id": "6", "ReceiptHandle": 5},
            {"Id": "7", "ReceiptHandle": "r-3", "Body": "ignored"},
        ]),
    );
    assert_eq!(r["ok"], true);
    assert_eq!(r["Successful"], json!([{"Id": "1"}, {"Id": "7"}]));
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
            ("3", "ReceiptHandleExpired"),
            ("4", "ReceiptHandleInvalid"),
            ("5", "MissingParameter"),
            ("6", "InvalidParameterValue"),
        ]
    );
    let counts = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}))["Counts"].clone();
    assert_eq!(counts, json!({"Visible": 0, "InFlight": 1, "Delayed": 0}));
}

#[test]
fn all_entries_failing_is_still_a_success() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let r = delete_batch(&mut h, "q", json!([{"Id": "a", "ReceiptHandle": "r-1"}]));
    assert_eq!(r["ok"], true);
    assert_eq!(r["Successful"], json!([]));
    assert_eq!(r["Failed"][0]["error"]["code"], "ReceiptHandleInvalid");
}
