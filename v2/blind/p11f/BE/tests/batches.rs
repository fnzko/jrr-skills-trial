mod common;

use common::Harness;
use serde_json::{Value, json};

fn create(h: &mut Harness, name: &str, attributes: Value) {
    h.ok(json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attributes}));
}

fn send_batch(h: &mut Harness, entries: Value) -> Value {
    h.run(json!({"Command": "SendMessageBatch", "QueueName": "q", "Entries": entries}))
}

fn delete_batch(h: &mut Harness, entries: Value) -> Value {
    h.run(json!({"Command": "DeleteMessageBatch", "QueueName": "q", "Entries": entries}))
}

fn code(response: &Value) -> &str {
    assert_eq!(response["ok"], false, "{response}");
    response["error"]["code"]
        .as_str()
        .expect("code is a string")
}

#[test]
fn send_batch_assigns_ids_in_order() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let r = send_batch(
        &mut h,
        json!([{"Id": "a", "Body": "x"}, {"Id": "b", "Body": "y"}]),
    );
    assert_eq!(
        r,
        json!({"ok": true, "Failed": [],
            "Successful": [{"Id": "a", "MessageId": "m-1"}, {"Id": "b", "MessageId": "m-2"}]})
    );
}

#[test]
fn send_batch_on_fifo_sees_earlier_entries() {
    let mut h = Harness::new();
    create(&mut h, "q.fifo", json!({"Fifo": true}));
    let r = h.run(
        json!({"Command": "SendMessageBatch", "QueueName": "q.fifo", "Entries": [
            {"Id": "a", "Body": "x", "GroupId": "g", "DeduplicationId": "d"},
            {"Id": "b", "Body": "y", "GroupId": "g", "DeduplicationId": "d"},
            {"Id": "c", "Body": "z", "GroupId": "g", "DeduplicationId": "e"},
        ]}),
    );
    assert_eq!(
        r,
        json!({"ok": true, "Failed": [], "Successful": [
            {"Id": "a", "MessageId": "m-1", "SequenceNumber": 1},
            {"Id": "b", "MessageId": "m-1", "SequenceNumber": 1},
            {"Id": "c", "MessageId": "m-2", "SequenceNumber": 2},
        ]})
    );
}

#[test]
fn failed_entries_do_not_stop_the_others_or_use_ids() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({"MaxMessageBytes": 2}));
    let r = send_batch(
        &mut h,
        json!([
            {"Id": "a", "Body": "xxx"},
            {"Id": "b"},
            {"Id": "c", "Body": "ok", "GroupId": "g"},
            {"Id": "d", "Body": "ok", "DelaySeconds": 901},
            {"Id": "e", "Body": 5},
            {"Id": "f", "Body": "ok", "DelaySeconds": null, "Other": 1},
        ]),
    );
    assert_eq!(r["ok"], true);
    assert_eq!(r["Successful"], json!([{"Id": "f", "MessageId": "m-1"}]));
    let failed: Vec<(&str, &str)> = r["Failed"]
        .as_array()
        .expect("Failed is an array")
        .iter()
        .map(|f| {
            (
                f["Id"].as_str().expect("Id"),
                f["error"]["code"].as_str().expect("code"),
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
            ("e", "InvalidParameterValue"),
        ]
    );
    assert!(r["Failed"][0]["error"]["message"].is_string());
}

#[test]
fn message_limit_applies_within_a_batch() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    for _ in 0..10_000 {
        let entries: Vec<Value> = (0..10)
            .map(|i| json!({"Id": format!("e{i}"), "Body": "x"}))
            .collect();
        let r = send_batch(&mut h, json!(entries));
        assert_eq!(r["Failed"], json!([]));
    }
    let r = send_batch(&mut h, json!([{"Id": "a", "Body": "x"}]));
    assert_eq!(r["Failed"][0]["error"]["code"], "LimitExceeded");
    assert_eq!(r["Successful"], json!([]));
}

#[test]
fn delete_batch_uses_handles_in_order() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    send_batch(
        &mut h,
        json!([{"Id": "a", "Body": "x"}, {"Id": "b", "Body": "y"}]),
    );
    h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 2}));
    let r = delete_batch(
        &mut h,
        json!([
            {"Id": "1", "ReceiptHandle": "r-1"},
            {"Id": "2", "ReceiptHandle": "r-1"},
            {"Id": "3", "ReceiptHandle": "r-9"},
            {"Id": "4"},
            {"Id": "5", "ReceiptHandle": "r-2"},
        ]),
    );
    assert_eq!(r["Successful"], json!([{"Id": "1"}, {"Id": "5"}]));
    let codes: Vec<&Value> = r["Failed"]
        .as_array()
        .expect("Failed is an array")
        .iter()
        .map(|f| &f["error"]["code"])
        .collect();
    assert_eq!(
        codes,
        [
            "ReceiptHandleExpired",
            "ReceiptHandleInvalid",
            "MissingParameter"
        ]
    );
    let attrs = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(
        attrs["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn batch_rules_and_their_order() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let eleven: Vec<Value> = (0..11)
        .map(|i| json!({"Id": format!("e{i}"), "Body": "x"}))
        .collect();
    let ten: Vec<Value> = (0..10)
        .map(|i| json!({"Id": format!("e{i}"), "Body": "x"}))
        .collect();
    assert_eq!(code(&send_batch(&mut h, json!([]))), "EmptyBatchRequest");
    assert_eq!(
        code(&send_batch(&mut h, json!(eleven))),
        "TooManyEntriesInBatchRequest"
    );
    // Too many entries wins over an invalid element.
    let mut mixed = vec![json!(1); 11];
    mixed[0] = json!({"Id": "a"});
    assert_eq!(
        code(&send_batch(&mut h, json!(mixed))),
        "TooManyEntriesInBatchRequest"
    );
    assert_eq!(send_batch(&mut h, json!(ten))["ok"], true);
    let bad_ids = [
        json!(["x"]),
        json!([null]),
        json!([{"Body": "x"}]),
        json!([{"Id": null, "Body": "x"}]),
        json!([{"Id": 1, "Body": "x"}]),
        json!([{"Id": "", "Body": "x"}]),
        json!([{"Id": "a b", "Body": "x"}]),
        json!([{"Id": "a".repeat(81), "Body": "x"}]),
    ];
    for entries in bad_ids {
        assert_eq!(
            code(&send_batch(&mut h, entries.clone())),
            "InvalidBatchEntryId",
            "{entries}"
        );
    }
    assert_eq!(
        send_batch(&mut h, json!([{"Id": "a".repeat(80), "Body": "x"}]))["ok"],
        true
    );
    // An invalid Id wins over duplicate Ids.
    let r = send_batch(&mut h, json!([{"Id": "a"}, {"Id": "a"}, {"Id": "!"}]));
    assert_eq!(code(&r), "InvalidBatchEntryId");
    let r = send_batch(
        &mut h,
        json!([{"Id": "a", "Body": "x"}, {"Id": "a", "Body": "x"}]),
    );
    assert_eq!(code(&r), "BatchEntryIdsNotDistinct");
    // Ids are case-sensitive.
    assert_eq!(
        send_batch(
            &mut h,
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
        json!([{"Id": "a", "Body": "x"}, {"Id": "a", "Body": "x"}]),
    );
    assert_eq!(code(&r), "BatchEntryIdsNotDistinct");
    let r = send_batch(&mut h, json!([{"Id": "a", "Body": "x"}]));
    assert_eq!(r["Successful"], json!([{"Id": "a", "MessageId": "m-1"}]));
}

#[test]
fn request_level_errors_follow_the_check_order() {
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
            h.err(json!({"Command": command, "QueueName": "q", "Entries": "x"})),
            "InvalidParameterValue"
        );
        assert_eq!(
            h.err(json!({"Command": command, "QueueName": 1, "Entries": []})),
            "InvalidParameterValue"
        );
        // Queue lookup comes before the numbered batch rules.
        assert_eq!(
            h.err(json!({"Command": command, "QueueName": "q", "Entries": []})),
            "QueueDoesNotExist"
        );
    }
}

#[test]
fn delete_batch_rules_apply_too() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    assert_eq!(code(&delete_batch(&mut h, json!([]))), "EmptyBatchRequest");
    let r = delete_batch(&mut h, json!([{"Id": "a"}, {"Id": "a"}]));
    assert_eq!(code(&r), "BatchEntryIdsNotDistinct");
    let r = delete_batch(&mut h, json!([{"ReceiptHandle": "r-1"}]));
    assert_eq!(code(&r), "InvalidBatchEntryId");
}
