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

fn codes(response: &Value) -> Vec<(String, String)> {
    response["Failed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            assert!(f["error"]["message"].is_string());
            (
                f["Id"].as_str().unwrap().to_owned(),
                f["error"]["code"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

#[test]
fn send_batch_assigns_ids_in_order() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let r = send_batch(
        &mut h,
        "q",
        json!([{"Id": "a", "Body": "x"}, {"Id": "b", "Body": "y", "DelaySeconds": 5}]),
    );
    assert_eq!(
        r,
        json!({"ok": true, "Failed": [], "Successful": [
            {"Id": "a", "MessageId": "m-1"}, {"Id": "b", "MessageId": "m-2"}]})
    );
    let r = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(
        r["Counts"],
        json!({"Visible": 1, "InFlight": 0, "Delayed": 1})
    );
}

#[test]
fn entries_fail_independently_and_see_earlier_entries() {
    let mut h = Harness::new();
    create(&mut h, "q.fifo", json!({"Fifo": true}));
    let r = send_batch(
        &mut h,
        "q.fifo",
        json!([
            {"Id": "1", "Body": "x", "GroupId": "g", "DeduplicationId": "d"},
            {"Id": "2", "Body": "x", "GroupId": "g", "DeduplicationId": "d"},
            {"Id": "3", "Body": "x", "DeduplicationId": "e"},
            {"Id": "4", "GroupId": "g", "DeduplicationId": "e"},
            {"Id": "5", "Body": "x", "GroupId": "g", "DeduplicationId": "e", "DelaySeconds": 1},
            {"Id": "6", "Body": 5, "GroupId": "g", "DeduplicationId": "e"},
            {"Id": "7", "Body": "y", "GroupId": "g", "DeduplicationId": "f"},
        ]),
    );
    assert_eq!(
        r["Successful"],
        json!([
            {"Id": "1", "MessageId": "m-1", "SequenceNumber": 1},
            {"Id": "2", "MessageId": "m-1", "SequenceNumber": 1},
            {"Id": "7", "MessageId": "m-2", "SequenceNumber": 2},
        ])
    );
    let failed: Vec<(&str, &str)> = [
        ("3", "MissingParameter"),
        ("4", "MissingParameter"),
        ("5", "UnsupportedOperation"),
        ("6", "InvalidParameterValue"),
    ]
    .to_vec();
    let got = codes(&r);
    let got: Vec<(&str, &str)> = got.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
    assert_eq!(got, failed);
}

#[test]
fn entries_obey_queue_size_limit_and_message_limit() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({"MaxMessageBytes": 2}));
    let r = send_batch(
        &mut h,
        "q",
        json!([{"Id": "a", "Body": "abc"}, {"Id": "b", "Body": "ab"}]),
    );
    assert_eq!(
        codes(&r),
        vec![("a".to_owned(), "InvalidParameterValue".to_owned())]
    );
    assert_eq!(r["Successful"], json!([{"Id": "b", "MessageId": "m-1"}]));
}

#[test]
fn batch_hits_the_message_limit_midway() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let entries: Vec<Value> = (0..10)
        .map(|i| json!({"Id": format!("e{i}"), "Body": "x"}))
        .collect();
    for _ in 0..9999 {
        send_batch(&mut h, "q", Value::Array(entries.clone()));
    }
    // 99990 messages held: 10 more fit, the next batch's first entry is refused.
    let r = send_batch(&mut h, "q", Value::Array(entries.clone()));
    assert_eq!(r["Successful"].as_array().unwrap().len(), 10);
    let r = send_batch(&mut h, "q", json!([{"Id": "x", "Body": "x"}]));
    assert_eq!(
        codes(&r),
        vec![("x".to_owned(), "LimitExceeded".to_owned())]
    );
}

#[test]
fn delete_batch_uses_handles_in_order() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    create(&mut h, "other", json!({}));
    send_batch(
        &mut h,
        "q",
        json!([{"Id": "a", "Body": "x"}, {"Id": "b", "Body": "y"}]),
    );
    h.ok(json!({"Command": "SendMessage", "QueueName": "other", "Body": "z"}));
    h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 2}));
    h.ok(json!({"Command": "ReceiveMessage", "QueueName": "other"}));
    let r = delete_batch(
        &mut h,
        "q",
        json!([
            {"Id": "1", "ReceiptHandle": "r-1"},
            {"Id": "2", "ReceiptHandle": "r-1"},
            {"Id": "3", "ReceiptHandle": "r-01"},
            {"Id": "4", "ReceiptHandle": "r-3"},
            {"Id": "5"},
            {"Id": "6", "ReceiptHandle": 1},
            {"Id": "7", "ReceiptHandle": "r-2"},
        ]),
    );
    assert_eq!(r["Successful"], json!([{"Id": "1"}, {"Id": "7"}]));
    let got = codes(&r);
    let got: Vec<(&str, &str)> = got.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
    assert_eq!(
        got,
        vec![
            ("2", "ReceiptHandleExpired"),
            ("3", "ReceiptHandleInvalid"),
            ("4", "ReceiptHandleExpired"),
            ("5", "MissingParameter"),
            ("6", "InvalidParameterValue"),
        ]
    );
    let r = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(
        r["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn request_level_errors_do_nothing() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let too_many: Vec<Value> = (0..11)
        .map(|i| json!({"Id": format!("e{i}"), "Body": "x"}))
        .collect();
    let cases = [
        (json!({"QueueName": "q"}), "MissingParameter"),
        (json!({"Entries": []}), "MissingParameter"),
        (
            json!({"QueueName": "q", "Entries": null}),
            "MissingParameter",
        ),
        (
            json!({"QueueName": "q", "Entries": {}}),
            "InvalidParameterValue",
        ),
        (
            json!({"QueueName": 1, "Entries": []}),
            "InvalidParameterValue",
        ),
        // The lookup comes before the batch rules.
        (
            json!({"QueueName": "nope", "Entries": []}),
            "QueueDoesNotExist",
        ),
        (
            json!({"QueueName": "q", "Entries": []}),
            "EmptyBatchRequest",
        ),
        (
            json!({"QueueName": "q", "Entries": too_many}),
            "TooManyEntriesInBatchRequest",
        ),
        (
            json!({"QueueName": "q", "Entries": [{"Id": "a", "Body": "x"}, 5]}),
            "InvalidBatchEntryId",
        ),
        (
            json!({"QueueName": "q", "Entries": [{"Body": "x"}]}),
            "InvalidBatchEntryId",
        ),
        (
            json!({"QueueName": "q", "Entries": [{"Id": null, "Body": "x"}]}),
            "InvalidBatchEntryId",
        ),
        (
            json!({"QueueName": "q", "Entries": [{"Id": 1, "Body": "x"}]}),
            "InvalidBatchEntryId",
        ),
        (
            json!({"QueueName": "q", "Entries": [{"Id": "a b", "Body": "x"}]}),
            "InvalidBatchEntryId",
        ),
        (
            json!({"QueueName": "q", "Entries": [{"Id": "a".repeat(81), "Body": "x"}]}),
            "InvalidBatchEntryId",
        ),
        (
            json!({"QueueName": "q", "Entries": [{"Id": "a", "Body": "x"}, {"Id": "a", "Body": "y"}]}),
            "BatchEntryIdsNotDistinct",
        ),
        // An invalid Id (rule 3) beats duplicate ids (rule 4).
        (
            json!({"QueueName": "q", "Entries": [{"Id": "a"}, {"Id": "a"}, {"Id": ""}]}),
            "InvalidBatchEntryId",
        ),
    ];
    for command in ["SendMessageBatch", "DeleteMessageBatch"] {
        for (members, code) in &cases {
            let mut request = members.clone();
            request["Command"] = json!(command);
            assert_eq!(h.err(request.clone()), *code, "{request}");
        }
    }
    let r = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(
        r["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
    // Ten entries and an `Id` of 80 characters are fine.
    let ten = send_batch(&mut h, "q", json!([{"Id": "a".repeat(80), "Body": "x"}]));
    assert_eq!(
        ten["Successful"],
        json!([{"Id": "a".repeat(80), "MessageId": "m-1"}])
    );
    let r = send_batch(&mut h, "q", Value::Array(too_many[..10].to_vec()));
    assert_eq!(r["Successful"].as_array().unwrap().len(), 10);
}

#[test]
fn extra_entry_members_are_ignored() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let r = send_batch(
        &mut h,
        "q",
        json!([{"Id": "a", "Body": "x", "Junk": [1], "GroupId": null}]),
    );
    assert_eq!(r["Successful"], json!([{"Id": "a", "MessageId": "m-1"}]));
}
