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

fn visible(h: &mut Harness, queue: &str) -> Value {
    h.ok(json!({"Command": "GetQueueAttributes", "QueueName": queue}))["Counts"]["Visible"].clone()
}

#[test]
fn send_batch_reports_results_in_entry_order() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let r = send_batch(
        &mut h,
        "q",
        json!([
            {"Id": "a", "Body": "one", "Other": 1},
            {"Id": "b"},
            {"Id": "c", "Body": "three", "DelaySeconds": 5},
            {"Id": "d", "Body": "x", "GroupId": "g"},
            {"Id": "e", "Body": "five", "DelaySeconds": null},
        ]),
    );
    assert_eq!(
        r,
        json!({
            "ok": true,
            "Successful": [
                {"Id": "a", "MessageId": "m-1"},
                {"Id": "c", "MessageId": "m-2"},
                {"Id": "e", "MessageId": "m-3"},
            ],
            "Failed": [
                {"Id": "b", "error": {"code": "MissingParameter", "message": r["Failed"][0]["error"]["message"].clone()}},
                {"Id": "d", "error": {"code": "UnsupportedOperation", "message": r["Failed"][1]["error"]["message"].clone()}},
            ],
        })
    );
    assert!(r["Failed"][0]["error"]["message"].is_string());
    let counts = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}))["Counts"].clone();
    assert_eq!(counts, json!({"Visible": 2, "InFlight": 0, "Delayed": 1}));
}

#[test]
fn send_batch_entry_value_errors() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({"MaxMessageBytes": 3}));
    let r = send_batch(
        &mut h,
        "q",
        json!([
            {"Id": "1", "Body": 5},
            {"Id": "2", "Body": ""},
            {"Id": "3", "Body": "long"},
            {"Id": "4", "Body": "ok", "DelaySeconds": 901},
            {"Id": "5", "Body": "ok"},
        ]),
    );
    let codes: Vec<&str> = r["Failed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["error"]["code"].as_str().unwrap())
        .collect();
    assert_eq!(codes, ["InvalidParameterValue"; 4]);
    assert_eq!(r["Successful"], json!([{"Id": "5", "MessageId": "m-1"}]));
}

#[test]
fn send_batch_entries_see_earlier_entries_on_fifo_queues() {
    let mut h = Harness::new();
    create(&mut h, "q.fifo", json!({"Fifo": true}));
    let r = send_batch(
        &mut h,
        "q.fifo",
        json!([
            {"Id": "a", "Body": "x", "GroupId": "g", "DeduplicationId": "d"},
            {"Id": "b", "Body": "y", "GroupId": "g", "DeduplicationId": "d"},
            {"Id": "c", "Body": "z", "GroupId": "g", "DeduplicationId": "e"},
            {"Id": "d", "Body": "w", "DeduplicationId": "f"},
            {"Id": "e", "Body": "v", "GroupId": "g", "DeduplicationId": "f", "DelaySeconds": 1},
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
    let codes: Vec<&Value> = r["Failed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| &f["error"]["code"])
        .collect();
    assert_eq!(codes, ["MissingParameter", "UnsupportedOperation"]);
    assert_eq!(r["Failed"][0]["Id"], "d");
    assert_eq!(r["Failed"][1]["Id"], "e");
}

#[test]
fn send_batch_sees_the_message_limit() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let entries: Vec<Value> = (0..10)
        .map(|i| json!({"Id": format!("i{i}"), "Body": "x"}))
        .collect();
    for _ in 0..9999 {
        send_batch(&mut h, "q", json!(entries));
    }
    // 99990 messages: 7 more fit, then the queue holds 99997.
    let r = send_batch(&mut h, "q", json!(&entries[..7]));
    assert_eq!(r["Successful"].as_array().unwrap().len(), 7);
    // Of the next 10 entries, the first 3 fit and the other 7 hit the limit.
    let r = send_batch(&mut h, "q", json!(entries));
    let ids: Vec<&str> = r["Successful"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["Id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["i0", "i1", "i2"]);
    assert_eq!(r["Failed"].as_array().unwrap().len(), 7);
    assert_eq!(r["Failed"][0]["Id"], "i3");
    assert_eq!(r["Failed"][0]["error"]["code"], "LimitExceeded");
}

#[test]
fn batch_request_errors_do_nothing() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let body = |id: &str| json!({"Id": id, "Body": "x"});
    let long_id = "a".repeat(81);
    let cases = [
        (json!([]), "EmptyBatchRequest"),
        (
            (0..11).map(|i| body(&format!("i{i}"))).collect::<Value>(),
            "TooManyEntriesInBatchRequest",
        ),
        (json!([body("a"), "text"]), "InvalidBatchEntryId"),
        (json!([[]]), "InvalidBatchEntryId"),
        (json!([{"Body": "x"}]), "InvalidBatchEntryId"),
        (json!([{"Id": null, "Body": "x"}]), "InvalidBatchEntryId"),
        (json!([{"Id": 1, "Body": "x"}]), "InvalidBatchEntryId"),
        (json!([{"Id": "", "Body": "x"}]), "InvalidBatchEntryId"),
        (json!([{"Id": "a b", "Body": "x"}]), "InvalidBatchEntryId"),
        (json!([{"Id": long_id, "Body": "x"}]), "InvalidBatchEntryId"),
        // Rule 3 comes before rule 4.
        (
            json!([body("a"), body("a"), {"Id": "!"}]),
            "InvalidBatchEntryId",
        ),
        (
            json!([body("a"), body("b"), body("a")]),
            "BatchEntryIdsNotDistinct",
        ),
        (json!([body("A"), body("a")]), "ok"),
        (json!([body(&"a".repeat(80))]), "ok"),
    ];
    for (entries, code) in cases {
        for command in ["SendMessageBatch", "DeleteMessageBatch"] {
            let r = h.run(json!({"Command": command, "QueueName": "q", "Entries": entries}));
            if code == "ok" {
                assert_eq!(r["ok"], true, "{command} {entries}");
            } else {
                assert_eq!(r["error"]["code"], code, "{command} {entries}");
            }
        }
    }
    // Only the "ok" cases of SendMessageBatch sent anything.
    assert_eq!(visible(&mut h, "q"), 3);
}

#[test]
fn batch_check_order() {
    let mut h = Harness::new();
    for command in ["SendMessageBatch", "DeleteMessageBatch"] {
        assert_eq!(
            h.err(json!({"Command": command, "QueueName": "q"})),
            "MissingParameter"
        );
        assert_eq!(
            h.err(json!({"Command": command, "Entries": []})),
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
        // The queue lookup comes before the numbered rules.
        assert_eq!(
            h.err(json!({"Command": command, "QueueName": "q", "Entries": []})),
            "QueueDoesNotExist"
        );
    }
}

#[test]
fn delete_batch_handles_each_entry() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    create(&mut h, "other", json!({}));
    send_batch(
        &mut h,
        "q",
        json!([{"Id": "a", "Body": "1"}, {"Id": "b", "Body": "2"}]),
    );
    h.ok(json!({"Command": "SendMessage", "QueueName": "other", "Body": "3"}));
    let got = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 2}));
    assert_eq!(got["Messages"][0]["ReceiptHandle"], "r-1");
    let foreign = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "other"}));
    assert_eq!(foreign["Messages"][0]["ReceiptHandle"], "r-3");
    let r = delete_batch(
        &mut h,
        "q",
        json!([
            {"Id": "1", "ReceiptHandle": "r-1"},
            {"Id": "2", "ReceiptHandle": "r-1"},   // used by the entry before
            {"Id": "3", "ReceiptHandle": "r-99"},
            {"Id": "4"},
            {"Id": "5", "ReceiptHandle": 7},
            {"Id": "6", "ReceiptHandle": "r-3"},   // another queue's message
            {"Id": "7", "ReceiptHandle": "r-2"},
        ]),
    );
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
            ("3", "ReceiptHandleInvalid"),
            ("4", "MissingParameter"),
            ("5", "InvalidParameterValue"),
            ("6", "ReceiptHandleExpired"),
        ]
    );
    let counts = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}))["Counts"].clone();
    assert_eq!(counts, json!({"Visible": 0, "InFlight": 0, "Delayed": 0}));
    // The success response has both members even when every entry failed.
    let r = delete_batch(&mut h, "q", json!([{"Id": "x", "ReceiptHandle": "r-1"}]));
    assert_eq!(r["Successful"], json!([]));
    assert_eq!(r["Failed"].as_array().unwrap().len(), 1);
}
