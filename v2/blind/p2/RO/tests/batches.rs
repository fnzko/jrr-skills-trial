mod common;

use common::Harness;
use serde_json::{Value, json};

fn send_batch(h: &mut Harness, queue: &str, entries: Value) -> Value {
    h.ok(json!({"Command": "SendMessageBatch", "QueueName": queue, "Entries": entries}))
}

fn setup(attributes: Value) -> Harness {
    let mut h = Harness::new();
    let name = if attributes["Fifo"] == true {
        "q.fifo"
    } else {
        "q"
    };
    h.ok(json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attributes}));
    h
}

fn codes(response: &Value) -> Vec<(&str, &str)> {
    response["Failed"]
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

#[test]
fn send_batch_assigns_ids_in_entry_order() {
    let mut h = setup(json!({}));
    let r = send_batch(
        &mut h,
        "q",
        json!([{"Id": "a", "Body": "x"}, {"Id": "b", "Body": "y", "DelaySeconds": 5}]),
    );
    assert_eq!(
        r["Successful"],
        json!([{"Id": "a", "MessageId": "m-1"}, {"Id": "b", "MessageId": "m-2"}])
    );
    assert_eq!(r["Failed"], json!([]));
    let c = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(
        c["Counts"],
        json!({"Visible": 1, "InFlight": 0, "Delayed": 1})
    );
}

#[test]
fn failed_entries_do_not_stop_the_others_or_use_message_ids() {
    let mut h = setup(json!({"MaxMessageBytes": 3}));
    let r = send_batch(
        &mut h,
        "q",
        json!([
            {"Id": "a", "Body": "ok"},
            {"Id": "b", "Body": "toolong"},
            {"Id": "c"},
            {"Id": "d", "Body": "x", "GroupId": "g"},
            {"Id": "e", "Body": "", "DelaySeconds": 1},
            {"Id": "f", "Body": "x", "DelaySeconds": 901},
            {"Id": "g", "Body": "fine", "DelaySeconds": null},
            {"Id": "h", "Body": "yes"},
        ]),
    );
    assert_eq!(
        r["Successful"],
        json!([{"Id": "a", "MessageId": "m-1"}, {"Id": "h", "MessageId": "m-2"}])
    );
    assert_eq!(
        codes(&r),
        [
            ("b", "InvalidParameterValue"),
            ("c", "MissingParameter"),
            ("d", "UnsupportedOperation"),
            ("e", "InvalidParameterValue"),
            ("f", "InvalidParameterValue"),
            ("g", "InvalidParameterValue"),
        ]
    );
    assert!(r["Failed"][0]["error"]["message"].is_string());
}

#[test]
fn fifo_batch_sees_earlier_entries() {
    let mut h = setup(json!({"Fifo": true}));
    let r = send_batch(
        &mut h,
        "q.fifo",
        json!([
            {"Id": "a", "Body": "x", "GroupId": "g", "DeduplicationId": "d"},
            {"Id": "b", "Body": "y", "GroupId": "g", "DeduplicationId": "d"},
            {"Id": "c", "Body": "z", "GroupId": "g", "DeduplicationId": "e"},
            {"Id": "d", "Body": "z", "DeduplicationId": "f"},
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
    assert_eq!(codes(&r), [("d", "MissingParameter")]);
}

#[test]
fn send_batch_sees_the_message_limit() {
    let mut h = setup(json!({}));
    for _ in 0..10_000 {
        let entries: Vec<Value> = (0..10)
            .map(|i| json!({"Id": format!("e{i}"), "Body": "x"}))
            .collect();
        let r = send_batch(&mut h, "q", Value::Array(entries));
        assert_eq!(r["Failed"], json!([]));
    }
    let r = send_batch(
        &mut h,
        "q",
        json!([{"Id": "a", "Body": "x"}, {"Id": "b", "Body": "x"}]),
    );
    assert_eq!(r["Successful"], json!([]));
    assert_eq!(codes(&r), [("a", "LimitExceeded"), ("b", "LimitExceeded")]);
}

#[test]
fn batch_rules_and_check_order() {
    let mut h = setup(json!({}));
    let e = |entries: Value| h_err(entries);
    fn h_err(entries: Value) -> Value {
        json!({"Command": "SendMessageBatch", "QueueName": "q", "Entries": entries})
    }
    let long_id = "i".repeat(81);
    let too_many: Vec<Value> = (0..11)
        .map(|i| json!({"Id": format!("e{i}"), "Body": "x"}))
        .collect();
    let cases = [
        (e(json!([])), "EmptyBatchRequest"),
        (e(Value::Array(too_many)), "TooManyEntriesInBatchRequest"),
        (e(json!(["x"])), "InvalidBatchEntryId"),
        (e(json!([{"Body": "x"}])), "InvalidBatchEntryId"),
        (e(json!([{"Id": null, "Body": "x"}])), "InvalidBatchEntryId"),
        (e(json!([{"Id": 1, "Body": "x"}])), "InvalidBatchEntryId"),
        (e(json!([{"Id": "", "Body": "x"}])), "InvalidBatchEntryId"),
        (
            e(json!([{"Id": "a b", "Body": "x"}])),
            "InvalidBatchEntryId",
        ),
        (
            e(json!([{"Id": long_id, "Body": "x"}])),
            "InvalidBatchEntryId",
        ),
        (
            e(json!([{"Id": "a"}, {"Id": "a"}])),
            "BatchEntryIdsNotDistinct",
        ),
        // Rule 3 beats rule 4.
        (
            e(json!([{"Id": "a"}, {"Id": "a"}, 5])),
            "InvalidBatchEntryId",
        ),
        (
            json!({"Command": "SendMessageBatch", "QueueName": "q"}),
            "MissingParameter",
        ),
        (
            json!({"Command": "SendMessageBatch", "Entries": []}),
            "MissingParameter",
        ),
        (
            json!({"Command": "SendMessageBatch", "QueueName": "q", "Entries": {}}),
            "InvalidParameterValue",
        ),
        (
            json!({"Command": "SendMessageBatch", "QueueName": 1, "Entries": []}),
            "InvalidParameterValue",
        ),
        // The lookup beats the numbered rules.
        (
            json!({"Command": "SendMessageBatch", "QueueName": "nope", "Entries": []}),
            "QueueDoesNotExist",
        ),
        (
            json!({"Command": "DeleteMessageBatch", "QueueName": "q", "Entries": []}),
            "EmptyBatchRequest",
        ),
        (
            json!({"Command": "DeleteMessageBatch", "QueueName": "nope", "Entries": [1]}),
            "QueueDoesNotExist",
        ),
        (
            json!({"Command": "DeleteMessageBatch", "QueueName": "q"}),
            "MissingParameter",
        ),
    ];
    for (command, code) in cases {
        assert_eq!(h.err(command.clone()), code, "{command}");
    }
    // A failed request does nothing: the first message id is still 1.
    let r = send_batch(&mut h, "q", json!([{"Id": "a", "Body": "x"}]));
    assert_eq!(r["Successful"][0]["MessageId"], "m-1");
    // Ten entries and an 80-character Id are fine.
    let ten: Vec<Value> = (0..10)
        .map(|i| json!({"Id": format!("{}{i}", "i".repeat(79)), "Body": "x"}))
        .collect();
    assert_eq!(
        send_batch(&mut h, "q", Value::Array(ten))["Successful"]
            .as_array()
            .unwrap()
            .len(),
        10
    );
}

#[test]
fn delete_batch_handles_each_handle_in_order() {
    let mut h = setup(json!({}));
    send_batch(
        &mut h,
        "q",
        json!([{"Id": "a", "Body": "x"}, {"Id": "b", "Body": "y"}]),
    );
    let r = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 2}));
    let (h1, h2) = (
        r["Messages"][0]["ReceiptHandle"].clone(),
        r["Messages"][1]["ReceiptHandle"].clone(),
    );
    let r = h.ok(
        json!({"Command": "DeleteMessageBatch", "QueueName": "q", "Entries": [
            {"Id": "one", "ReceiptHandle": h1},
            {"Id": "again", "ReceiptHandle": h1},
            {"Id": "never", "ReceiptHandle": "r-99"},
            {"Id": "missing"},
            {"Id": "bad", "ReceiptHandle": 7},
            {"Id": "two", "ReceiptHandle": h2},
        ]}),
    );
    assert_eq!(r["Successful"], json!([{"Id": "one"}, {"Id": "two"}]));
    assert_eq!(
        codes(&r),
        [
            ("again", "ReceiptHandleExpired"),
            ("never", "ReceiptHandleInvalid"),
            ("missing", "MissingParameter"),
            ("bad", "InvalidParameterValue"),
        ]
    );
    let c = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(
        c["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn all_entries_failing_is_still_a_success() {
    let mut h = setup(json!({}));
    let r = h.run(
        json!({"Command": "DeleteMessageBatch", "QueueName": "q", "Entries": [
        {"Id": "a", "ReceiptHandle": "r-1"}]}),
    );
    assert_eq!(r["ok"], true);
    assert_eq!(r["Successful"], json!([]));
    assert_eq!(codes(&r), [("a", "ReceiptHandleInvalid")]);
}
