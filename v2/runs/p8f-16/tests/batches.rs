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

fn receive(h: &mut Harness, queue: &str, max: u64) -> Vec<Value> {
    let r = h.ok(json!({"Command": "ReceiveMessage", "QueueName": queue, "MaxMessages": max}));
    r["Messages"].as_array().unwrap().clone()
}

#[test]
fn send_batch_on_standard_queue() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let r = send_batch(
        &mut h,
        "q",
        json!([{"Id": "a", "Body": "x"}, {"Id": "b", "Body": "y"}]),
    );
    assert_eq!(
        r,
        json!({"ok": true, "Failed": [],
               "Successful": [{"Id": "a", "MessageId": "m-1"}, {"Id": "b", "MessageId": "m-2"}]})
    );
    let bodies: Vec<_> = receive(&mut h, "q", 10)
        .iter()
        .map(|m| m["Body"].clone())
        .collect();
    assert_eq!(bodies, [json!("x"), json!("y")]);
}

#[test]
fn send_batch_on_fifo_queue_with_dedup_inside_the_batch() {
    let mut h = Harness::new();
    create(&mut h, "q.fifo", json!({"Fifo": true}));
    let entry = |id: &str, dedup: &str| json!({"Id": id, "Body": "b", "GroupId": "g", "DeduplicationId": dedup});
    let r = send_batch(
        &mut h,
        "q.fifo",
        json!([entry("a", "d1"), entry("b", "d1"), entry("c", "d2")]),
    );
    assert_eq!(
        r["Successful"],
        json!([
            {"Id": "a", "MessageId": "m-1", "SequenceNumber": 1},
            {"Id": "b", "MessageId": "m-1", "SequenceNumber": 1},
            {"Id": "c", "MessageId": "m-2", "SequenceNumber": 2},
        ])
    );
    assert_eq!(r["Failed"], json!([]));
}

#[test]
fn failed_entries_do_not_stop_the_others() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({"MaxMessageBytes": 3}));
    let r = send_batch(
        &mut h,
        "q",
        json!([
            {"Id": "ok1", "Body": "a"},
            {"Id": "nobody"},
            {"Id": "big", "Body": "abcd"},
            {"Id": "badtype", "Body": 5},
            {"Id": "nulldelay", "Body": "b", "DelaySeconds": null},
            {"Id": "badgroup", "Body": "c", "GroupId": "g"},
            {"Id": "baddelay", "Body": "c", "DelaySeconds": 901},
            {"Id": "empty", "Body": ""},
        ]),
    );
    assert_eq!(r["ok"], true);
    assert_eq!(
        r["Successful"],
        json!([{"Id": "ok1", "MessageId": "m-1"}, {"Id": "nulldelay", "MessageId": "m-2"}])
    );
    let failed: Vec<(&str, &str)> = r["Failed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            assert!(f["error"]["message"].is_string());
            assert_eq!(f.as_object().unwrap().len(), 2);
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
            ("big", "InvalidParameterValue"),
            ("badtype", "InvalidParameterValue"),
            ("badgroup", "UnsupportedOperation"),
            ("baddelay", "InvalidParameterValue"),
            ("empty", "InvalidParameterValue"),
        ]
    );
    assert_eq!(receive(&mut h, "q", 10).len(), 2);
}

#[test]
fn entry_checks_follow_the_single_command_order() {
    let mut h = Harness::new();
    create(&mut h, "q.fifo", json!({"Fifo": true}));
    let r = send_batch(
        &mut h,
        "q.fifo",
        json!([
            {"Id": "delay", "Body": "a", "GroupId": "g", "DeduplicationId": "1", "DelaySeconds": 5},
            {"Id": "nogroup", "Body": "a", "DeduplicationId": "2"},
            {"Id": "nodedup", "Body": "a", "GroupId": "g"},
        ]),
    );
    let codes: Vec<_> = r["Failed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["error"]["code"].clone())
        .collect();
    assert_eq!(
        codes,
        [
            json!("UnsupportedOperation"),
            json!("MissingParameter"),
            json!("MissingParameter")
        ]
    );
}

#[test]
fn entry_queue_name_is_ignored() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let r = send_batch(
        &mut h,
        "q",
        json!([{"Id": "a", "Body": "x", "QueueName": "other", "Command": 7}]),
    );
    assert_eq!(r["Successful"], json!([{"Id": "a", "MessageId": "m-1"}]));
}

#[test]
fn message_limit_applies_across_entries() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    for _ in 0..10_000 {
        let entries: Vec<Value> = (0..10)
            .map(|i| json!({"Id": format!("e{i}"), "Body": "x"}))
            .collect();
        let r = send_batch(&mut h, "q", json!(entries));
        assert_eq!(r["Failed"], json!([]));
    }
    let r = send_batch(&mut h, "q", json!([{"Id": "a", "Body": "x"}]));
    assert_eq!(r["Failed"][0]["error"]["code"], "LimitExceeded");
    assert_eq!(r["Successful"], json!([]));
}

#[test]
fn request_level_rules() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    assert_eq!(
        code(&send_batch(&mut h, "q", json!([]))),
        "EmptyBatchRequest"
    );
    let eleven: Vec<Value> = (0..11)
        .map(|i| json!({"Id": format!("e{i}"), "Body": "x"}))
        .collect();
    assert_eq!(
        code(&send_batch(&mut h, "q", json!(eleven))),
        "TooManyEntriesInBatchRequest"
    );
    let ten: Vec<Value> = (0..10)
        .map(|i| json!({"Id": format!("e{i}"), "Body": "x"}))
        .collect();
    assert_eq!(send_batch(&mut h, "q", json!(ten))["ok"], true);

    // rule 2 wins over rule 3 and rule 3 over rule 4
    let mut many: Vec<Value> = (0..11).map(|_| json!(5)).collect();
    many[0] = json!({"Id": "a"});
    assert_eq!(
        code(&send_batch(&mut h, "q", json!(many))),
        "TooManyEntriesInBatchRequest"
    );
    let dup_and_bad = json!([{"Id": "a", "Body": "x"}, {"Id": "a", "Body": "x"}, {"Body": "x"}]);
    assert_eq!(
        code(&send_batch(&mut h, "q", dup_and_bad)),
        "InvalidBatchEntryId"
    );
    let dup = json!([{"Id": "a", "Body": "x"}, {"Id": "a", "Body": "y"}]);
    assert_eq!(
        code(&send_batch(&mut h, "q", dup)),
        "BatchEntryIdsNotDistinct"
    );
    assert_eq!(receive(&mut h, "q", 10).len(), 10);
}

#[test]
fn invalid_entry_ids() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    for bad in [
        json!(5),
        json!(null),
        json!("a"),
        json!([]),
        json!({"Body": "x"}),
        json!({"Id": null, "Body": "x"}),
        json!({"Id": 1, "Body": "x"}),
        json!({"Id": "", "Body": "x"}),
        json!({"Id": "a b", "Body": "x"}),
        json!({"Id": "a.b", "Body": "x"}),
        json!({"Id": "x".repeat(81), "Body": "x"}),
    ] {
        let r = send_batch(&mut h, "q", json!([{"Id": "ok", "Body": "x"}, bad.clone()]));
        assert_eq!(code(&r), "InvalidBatchEntryId", "{bad}");
    }
    let r = send_batch(
        &mut h,
        "q",
        json!([{"Id": "x".repeat(80), "Body": "x"}, {"Id": "A_-9", "Body": "x"}]),
    );
    assert_eq!(r["Failed"], json!([]));
    assert_eq!(receive(&mut h, "q", 10).len(), 2);
}

#[test]
fn check_order_before_the_batch_rules() {
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
        for bad in [json!({}), json!("a"), json!(1)] {
            let c = h.err(json!({"Command": command, "QueueName": "q", "Entries": bad}));
            assert_eq!(c, "InvalidParameterValue");
        }
        let c = h.err(json!({"Command": command, "QueueName": 5, "Entries": []}));
        assert_eq!(c, "InvalidParameterValue");
        let c = h.err(json!({"Command": command, "QueueName": "q", "Entries": []}));
        assert_eq!(c, "QueueDoesNotExist");
    }
}

#[test]
fn delete_batch_sees_earlier_entries() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    for body in ["a", "b", "c"] {
        h.ok(json!({"Command": "SendMessage", "QueueName": "q", "Body": body}));
    }
    let got = receive(&mut h, "q", 3);
    let handle = |i: usize| got[i]["ReceiptHandle"].clone();
    let r = delete_batch(
        &mut h,
        "q",
        json!([
            {"Id": "d0", "ReceiptHandle": handle(0)},
            {"Id": "again", "ReceiptHandle": handle(0)},
            {"Id": "never", "ReceiptHandle": "r-99"},
            {"Id": "none"},
            {"Id": "typed", "ReceiptHandle": 4},
            {"Id": "d2", "ReceiptHandle": handle(2)},
        ]),
    );
    assert_eq!(r["Successful"], json!([{"Id": "d0"}, {"Id": "d2"}]));
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
            ("typed", "InvalidParameterValue"),
        ]
    );
    let counts = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}))["Counts"].clone();
    assert_eq!(counts, json!({"Visible": 0, "InFlight": 1, "Delayed": 0}));
}

#[test]
fn delete_batch_rejects_handles_of_other_queues() {
    let mut h = Harness::new();
    create(&mut h, "a", json!({}));
    create(&mut h, "b", json!({}));
    h.ok(json!({"Command": "SendMessage", "QueueName": "a", "Body": "x"}));
    let got = receive(&mut h, "a", 1);
    let r = delete_batch(
        &mut h,
        "b",
        json!([{"Id": "x", "ReceiptHandle": got[0]["ReceiptHandle"]}]),
    );
    assert_eq!(r["Failed"][0]["error"]["code"], "ReceiptHandleExpired");
    assert_eq!(r["Successful"], json!([]));
}

#[test]
fn delete_batch_request_rules() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    assert_eq!(
        code(&delete_batch(&mut h, "q", json!([]))),
        "EmptyBatchRequest"
    );
    let eleven: Vec<Value> = (0..11).map(|i| json!({"Id": format!("e{i}")})).collect();
    assert_eq!(
        code(&delete_batch(&mut h, "q", json!(eleven))),
        "TooManyEntriesInBatchRequest"
    );
    assert_eq!(
        code(&delete_batch(&mut h, "q", json!([{"Id": 1}]))),
        "InvalidBatchEntryId"
    );
    let dup = json!([{"Id": "a", "ReceiptHandle": "r-1"}, {"Id": "a", "ReceiptHandle": "r-2"}]);
    assert_eq!(
        code(&delete_batch(&mut h, "q", dup)),
        "BatchEntryIdsNotDistinct"
    );
}
