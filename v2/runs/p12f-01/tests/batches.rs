mod common;

use common::Harness;
use serde_json::{Value, json};

fn harness() -> Harness {
    let mut h = Harness::new();
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    h
}

fn send_batch(h: &mut Harness, entries: Value) -> Value {
    h.ok(json!({"Command": "SendMessageBatch", "QueueName": "q", "Entries": entries}))
}

fn batch_code(h: &mut Harness, command: &str, entries: Value) -> String {
    h.err(json!({"Command": command, "QueueName": "q", "Entries": entries}))
}

fn receive_all(h: &mut Harness, queue: &str) -> Vec<Value> {
    let r = h.ok(json!({"Command": "ReceiveMessage", "QueueName": queue, "MaxMessages": 10}));
    r["Messages"].as_array().unwrap().clone()
}

#[test]
fn send_batch_assigns_ids_in_order() {
    let mut h = harness();
    let r = send_batch(
        &mut h,
        json!([{"Id": "a", "Body": "one"}, {"Id": "b", "Body": "two", "DelaySeconds": 5}]),
    );
    assert_eq!(
        r,
        json!({"ok": true, "Failed": [],
            "Successful": [{"Id": "a", "MessageId": "m-1"}, {"Id": "b", "MessageId": "m-2"}]})
    );
    let bodies: Vec<_> = receive_all(&mut h, "q")
        .iter()
        .map(|m| m["Body"].clone())
        .collect();
    assert_eq!(bodies, [json!("one")]);
}

#[test]
fn failing_entries_do_not_stop_the_others() {
    let mut h = harness();
    let r = send_batch(
        &mut h,
        json!([
            {"Id": "ok1", "Body": "x"},
            {"Id": "nobody"},
            {"Id": "toolong", "Body": "y".repeat(1025)},
            {"Id": "badgroup", "Body": "x", "GroupId": "g"},
            {"Id": "baddelay", "Body": "x", "DelaySeconds": 901},
            {"Id": "ok2", "Body": "z", "GroupId": null},
        ]),
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
            ("toolong", "InvalidParameterValue"),
            ("badgroup", "UnsupportedOperation"),
            ("baddelay", "InvalidParameterValue"),
        ]
    );
    assert!(r["Failed"][0]["error"]["message"].is_string());
    // A failed entry consumes no message id.
    assert_eq!(
        r["Successful"],
        json!([{"Id": "ok1", "MessageId": "m-1"}, {"Id": "ok2", "MessageId": "m-2"}])
    );
}

#[test]
fn batch_request_rules_are_checked_in_order() {
    let mut h = harness();
    let eleven: Vec<Value> = (0..11)
        .map(|n| json!({"Id": format!("e{n}"), "Body": "x"}))
        .collect();
    let cases = [
        (json!([]), "EmptyBatchRequest"),
        (Value::Array(eleven), "TooManyEntriesInBatchRequest"),
        (json!(["x"]), "InvalidBatchEntryId"),
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
        // Rule 3 beats rule 4.
        (
            json!([{"Id": "a", "Body": "x"}, {"Id": "a"}, {"Body": "x"}]),
            "InvalidBatchEntryId",
        ),
    ];
    for (entries, code) in cases {
        assert_eq!(
            batch_code(&mut h, "SendMessageBatch", entries.clone()),
            code,
            "{entries}"
        );
        assert_eq!(
            batch_code(&mut h, "DeleteMessageBatch", entries.clone()),
            code,
            "{entries}"
        );
    }
    assert_eq!(receive_all(&mut h, "q"), Vec::<Value>::new());
}

#[test]
fn ten_entries_and_an_id_of_80_characters_are_accepted() {
    let mut h = harness();
    let mut entries: Vec<Value> = (0..9)
        .map(|n| json!({"Id": format!("e{n}"), "Body": "x"}))
        .collect();
    entries.push(json!({"Id": "A".repeat(80), "Body": "x"}));
    let r = send_batch(&mut h, Value::Array(entries));
    assert_eq!(r["Successful"].as_array().unwrap().len(), 10);
}

#[test]
fn parameter_checks_come_before_rules() {
    let mut h = harness();
    let missing = |h: &mut Harness, c: &str, extra: Value| {
        let mut command = json!({"Command": c});
        command
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        h.err(command)
    };
    for c in ["SendMessageBatch", "DeleteMessageBatch"] {
        assert_eq!(
            missing(&mut h, c, json!({"QueueName": "q"})),
            "MissingParameter"
        );
        assert_eq!(
            missing(&mut h, c, json!({"Entries": []})),
            "MissingParameter"
        );
        assert_eq!(
            missing(&mut h, c, json!({"QueueName": "q", "Entries": "x"})),
            "InvalidParameterValue"
        );
        assert_eq!(
            missing(&mut h, c, json!({"QueueName": "q", "Entries": {}})),
            "InvalidParameterValue"
        );
        assert_eq!(
            missing(&mut h, c, json!({"QueueName": 1, "Entries": []})),
            "InvalidParameterValue"
        );
        assert_eq!(
            missing(&mut h, c, json!({"QueueName": "nope", "Entries": []})),
            "QueueDoesNotExist"
        );
    }
}

#[test]
fn entries_see_the_effects_of_earlier_entries_in_a_fifo_queue() {
    let mut h = Harness::new();
    h.ok(json!({"Command": "CreateQueue", "QueueName": "f.fifo", "Attributes": {"Fifo": true}}));
    let r = h.ok(
        json!({"Command": "SendMessageBatch", "QueueName": "f.fifo", "Entries": [
            {"Id": "a", "Body": "x", "GroupId": "g", "DeduplicationId": "d"},
            {"Id": "b", "Body": "y", "GroupId": "g", "DeduplicationId": "d"},
            {"Id": "c", "Body": "z", "GroupId": "g", "DeduplicationId": "e"},
            {"Id": "d", "Body": "w", "GroupId": "g", "DeduplicationId": "e", "DelaySeconds": 1},
            {"Id": "e", "Body": "w"},
        ]}),
    );
    assert_eq!(
        r["Successful"],
        json!([
            {"Id": "a", "MessageId": "m-1", "SequenceNumber": 1},
            {"Id": "b", "MessageId": "m-1", "SequenceNumber": 1},
            {"Id": "c", "MessageId": "m-2", "SequenceNumber": 2},
        ])
    );
    let codes: Vec<&str> = r["Failed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["error"]["code"].as_str().unwrap())
        .collect();
    assert_eq!(codes, ["UnsupportedOperation", "MissingParameter"]);
}

#[test]
fn max_message_bytes_applies_per_entry() {
    let mut h = Harness::new();
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    h.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q",
        "Attributes": {"MaxMessageBytes": 3}}));
    let r = send_batch(
        &mut h,
        json!([{"Id": "a", "Body": "abc"}, {"Id": "b", "Body": "abcd"}]),
    );
    assert_eq!(r["Successful"], json!([{"Id": "a", "MessageId": "m-1"}]));
    assert_eq!(r["Failed"][0]["Id"], "b");
    assert_eq!(r["Failed"][0]["error"]["code"], "InvalidParameterValue");
}

#[test]
fn delete_batch_uses_each_handle_once() {
    let mut h = harness();
    send_batch(
        &mut h,
        json!([{"Id": "a", "Body": "1"}, {"Id": "b", "Body": "2"}]),
    );
    let received = receive_all(&mut h, "q");
    let handles: Vec<Value> = received
        .iter()
        .map(|m| m["ReceiptHandle"].clone())
        .collect();
    assert_eq!(handles.len(), 2);
    let r = h.ok(
        json!({"Command": "DeleteMessageBatch", "QueueName": "q", "Entries": [
            {"Id": "first", "ReceiptHandle": handles[0]},
            {"Id": "again", "ReceiptHandle": handles[0]},
            {"Id": "never", "ReceiptHandle": "r-99"},
            {"Id": "none"},
            {"Id": "typed", "ReceiptHandle": 5},
            {"Id": "second", "ReceiptHandle": handles[1]},
        ]}),
    );
    assert_eq!(r["Successful"], json!([{"Id": "first"}, {"Id": "second"}]));
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
    assert_eq!(counts, json!({"Visible": 0, "InFlight": 0, "Delayed": 0}));
}

#[test]
fn delete_batch_handle_of_another_queue_is_expired() {
    let mut h = harness();
    h.ok(json!({"Command": "CreateQueue", "QueueName": "other"}));
    h.ok(json!({"Command": "SendMessage", "QueueName": "other", "Body": "x"}));
    let handle = receive_all(&mut h, "other")[0]["ReceiptHandle"].clone();
    let r = h.ok(json!({"Command": "DeleteMessageBatch", "QueueName": "q",
        "Entries": [{"Id": "a", "ReceiptHandle": handle}]}));
    assert_eq!(r["Successful"], json!([]));
    assert_eq!(r["Failed"][0]["error"]["code"], "ReceiptHandleExpired");
}
