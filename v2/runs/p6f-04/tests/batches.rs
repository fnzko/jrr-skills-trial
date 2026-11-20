mod common;

use common::Harness;
use serde_json::{Value, json};

fn create(h: &mut Harness, name: &str, attributes: Value) {
    h.ok(json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attributes}));
}

fn send_batch(h: &mut Harness, queue: &str, entries: Value) -> Value {
    h.ok(json!({"Command": "SendMessageBatch", "QueueName": queue, "Entries": entries}))
}

fn failed_codes(r: &Value) -> Vec<(&str, &str)> {
    r["Failed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            (
                f["Id"].as_str().unwrap(),
                f["error"]["code"].as_str().unwrap(),
            )
        })
        .collect()
}

fn receive(h: &mut Harness, queue: &str, max: u64) -> Vec<Value> {
    let r = h.ok(json!({"Command": "ReceiveMessage", "QueueName": queue, "MaxMessages": max}));
    r["Messages"].as_array().unwrap().clone()
}

#[test]
fn send_batch_assigns_ids_in_order_and_reports_each_entry() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let r = send_batch(
        &mut h,
        "q",
        json!([
            {"Id": "a", "Body": "one"},
            {"Id": "b", "Body": "two", "DelaySeconds": 5},
        ]),
    );
    assert_eq!(
        r["Successful"],
        json!([
            {"Id": "a", "MessageId": "m-1"},
            {"Id": "b", "MessageId": "m-2"},
        ])
    );
    assert_eq!(r["Failed"], json!([]));
    let bodies: Vec<_> = receive(&mut h, "q", 10)
        .iter()
        .map(|m| m["Body"].clone())
        .collect();
    assert_eq!(bodies, [json!("one")]);
}

#[test]
fn failed_entries_change_nothing_and_do_not_stop_the_others() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({"MaxMessageBytes": 3}));
    let r = send_batch(
        &mut h,
        "q",
        json!([
            {"Id": "ok1", "Body": "abc"},
            {"Id": "nobody"},
            {"Id": "toolong", "Body": "abcd"},
            {"Id": "badtype", "Body": 5},
            {"Id": "group", "Body": "x", "GroupId": "g"},
            {"Id": "ok2", "Body": "def", "Extra": true, "DelaySeconds": null},
        ]),
    );
    assert_eq!(
        r["Successful"],
        json!([
            {"Id": "ok1", "MessageId": "m-1"},
            {"Id": "ok2", "MessageId": "m-2"},
        ])
    );
    assert_eq!(
        failed_codes(&r),
        [
            ("nobody", "MissingParameter"),
            ("toolong", "InvalidParameterValue"),
            ("badtype", "InvalidParameterValue"),
            ("group", "UnsupportedOperation"),
        ]
    );
    assert!(r["Failed"][0]["error"]["message"].is_string());
}

#[test]
fn fifo_entries_see_the_effects_of_earlier_entries() {
    let mut h = Harness::new();
    create(&mut h, "f.fifo", json!({"Fifo": true}));
    let r = send_batch(
        &mut h,
        "f.fifo",
        json!([
            {"Id": "a", "Body": "x", "GroupId": "g", "DeduplicationId": "d1"},
            {"Id": "b", "Body": "y", "GroupId": "g", "DeduplicationId": "d1"},
            {"Id": "c", "Body": "z", "GroupId": "g", "DeduplicationId": "d2"},
            {"Id": "d", "Body": "w", "GroupId": "g"},
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
    assert_eq!(failed_codes(&r), [("d", "MissingParameter")]);
}

#[test]
fn entries_see_the_message_limit() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    for chunk in 0..10_000 {
        let entries: Vec<Value> = (0..10)
            .map(|i| json!({"Id": format!("e{i}"), "Body": "x"}))
            .collect();
        let r = send_batch(&mut h, "q", json!(entries));
        assert_eq!(r["Failed"], json!([]), "chunk {chunk}");
    }
    let r = send_batch(&mut h, "q", json!([{"Id": "a", "Body": "x"}]));
    assert_eq!(failed_codes(&r), [("a", "LimitExceeded")]);
}

#[test]
fn batch_rules_are_checked_in_order_and_do_nothing() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let ok = |id: &str| json!({"Id": id, "Body": "x"});
    let eleven: Vec<Value> = (0..11).map(|i| ok(&format!("e{i}"))).collect();
    let cases: Vec<(Value, &str)> = vec![
        (json!([]), "EmptyBatchRequest"),
        (json!(eleven), "TooManyEntriesInBatchRequest"),
        (json!([ok("a"), 5]), "InvalidBatchEntryId"),
        (json!([ok("a"), {"Body": "x"}]), "InvalidBatchEntryId"),
        (
            json!([ok("a"), {"Id": null, "Body": "x"}]),
            "InvalidBatchEntryId",
        ),
        (
            json!([ok("a"), {"Id": 1, "Body": "x"}]),
            "InvalidBatchEntryId",
        ),
        (
            json!([ok("a"), {"Id": "", "Body": "x"}]),
            "InvalidBatchEntryId",
        ),
        (
            json!([ok("a"), {"Id": "a b", "Body": "x"}]),
            "InvalidBatchEntryId",
        ),
        (json!([ok(&"i".repeat(81))]), "InvalidBatchEntryId"),
        (json!([ok("a"), ok("a")]), "BatchEntryIdsNotDistinct"),
        // Rule 3 comes before rule 4.
        (json!([ok("a"), ok("a"), 5]), "InvalidBatchEntryId"),
    ];
    for (entries, code) in cases {
        let got =
            h.err(json!({"Command": "SendMessageBatch", "QueueName": "q", "Entries": entries}));
        assert_eq!(got, code, "{entries}");
    }
    assert_eq!(receive(&mut h, "q", 10), Vec::<Value>::new());
    send_batch(&mut h, "q", json!([ok(&"i".repeat(80))]));
}

#[test]
fn check_order_before_the_batch_rules() {
    let mut h = Harness::new();
    let mut c = |v: Value| h.err(v);
    assert_eq!(
        c(json!({"Command": "SendMessageBatch", "Entries": []})),
        "MissingParameter"
    );
    assert_eq!(
        c(json!({"Command": "SendMessageBatch", "QueueName": "q"})),
        "MissingParameter"
    );
    assert_eq!(
        c(json!({"Command": "SendMessageBatch", "QueueName": "q", "Entries": {}})),
        "InvalidParameterValue"
    );
    assert_eq!(
        c(json!({"Command": "SendMessageBatch", "QueueName": 1, "Entries": []})),
        "InvalidParameterValue"
    );
    // The lookup comes before the numbered rules.
    assert_eq!(
        c(json!({"Command": "SendMessageBatch", "QueueName": "q", "Entries": []})),
        "QueueDoesNotExist"
    );
    assert_eq!(
        c(json!({"Command": "DeleteMessageBatch", "QueueName": "q", "Entries": []})),
        "QueueDoesNotExist"
    );
}

#[test]
fn delete_batch_reports_each_handle() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    create(&mut h, "other", json!({}));
    send_batch(
        &mut h,
        "q",
        json!([{"Id": "a", "Body": "1"}, {"Id": "b", "Body": "2"}]),
    );
    h.ok(json!({"Command": "SendMessage", "QueueName": "other", "Body": "3"}));
    let handles: Vec<Value> = receive(&mut h, "q", 2)
        .iter()
        .map(|m| m["ReceiptHandle"].clone())
        .collect();
    let foreign = receive(&mut h, "other", 1)[0]["ReceiptHandle"].clone();
    let r = h.ok(
        json!({"Command": "DeleteMessageBatch", "QueueName": "q", "Entries": [
            {"Id": "d1", "ReceiptHandle": handles[0]},
            {"Id": "again", "ReceiptHandle": handles[0]},
            {"Id": "never", "ReceiptHandle": "r-999"},
            {"Id": "foreign", "ReceiptHandle": foreign},
            {"Id": "missing"},
            {"Id": "typed", "ReceiptHandle": 1},
            {"Id": "d2", "ReceiptHandle": handles[1]},
        ]}),
    );
    assert_eq!(r["Successful"], json!([{"Id": "d1"}, {"Id": "d2"}]));
    assert_eq!(
        failed_codes(&r),
        [
            ("again", "ReceiptHandleExpired"),
            ("never", "ReceiptHandleInvalid"),
            ("foreign", "ReceiptHandleExpired"),
            ("missing", "MissingParameter"),
            ("typed", "InvalidParameterValue"),
        ]
    );
    let attrs = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(
        attrs["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn entry_queue_name_member_is_ignored() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let r = send_batch(
        &mut h,
        "q",
        json!([{"Id": "a", "Body": "x", "QueueName": "nope"}]),
    );
    assert_eq!(r["Failed"], json!([]));
    assert_eq!(r["Successful"], json!([{"Id": "a", "MessageId": "m-1"}]));
}
