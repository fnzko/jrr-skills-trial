mod common;

use common::Harness;
use serde_json::{Value, json};

fn send_batch(queue: &str, entries: Value) -> Value {
    json!({"Command": "SendMessageBatch", "QueueName": queue, "Entries": entries})
}

fn delete_batch(queue: &str, entries: Value) -> Value {
    json!({"Command": "DeleteMessageBatch", "QueueName": queue, "Entries": entries})
}

fn harness_with(name: &str, attributes: Value) -> Harness {
    let mut h = Harness::new();
    h.ok(json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attributes}));
    h
}

fn receive(h: &mut Harness, queue: &str, max: u64) -> Vec<Value> {
    let r = h.ok(json!({"Command": "ReceiveMessage", "QueueName": queue, "MaxMessages": max}));
    r["Messages"].as_array().unwrap().clone()
}

#[test]
fn send_batch_numbers_messages_in_entry_order() {
    let mut h = harness_with("q", json!({}));
    let r = h.ok(send_batch(
        "q",
        json!([{"Id": "a", "Body": "x"}, {"Id": "b", "Body": "y", "Extra": 1}]),
    ));
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
fn send_batch_entries_fail_independently_with_the_single_command_codes() {
    let mut h = harness_with("q", json!({"MaxMessageBytes": 3}));
    let r = h.ok(send_batch(
        "q",
        json!([
            {"Id": "ok1", "Body": "abc"},
            {"Id": "nobody"},
            {"Id": "nullbody", "Body": null},
            {"Id": "badtype", "Body": 5},
            {"Id": "empty", "Body": ""},
            {"Id": "toolong", "Body": "abcd"},
            {"Id": "group", "Body": "a", "GroupId": "g"},
            {"Id": "baddelay", "Body": "a", "DelaySeconds": 901},
            {"Id": "ok2", "Body": "d", "DelaySeconds": 5},
        ]),
    ));
    assert_eq!(
        r["Successful"],
        json!([{"Id": "ok1", "MessageId": "m-1"}, {"Id": "ok2", "MessageId": "m-2"}])
    );
    let failed: Vec<(String, String)> = r["Failed"]
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
        .collect();
    assert_eq!(
        failed,
        [
            ("nobody", "MissingParameter"),
            ("nullbody", "MissingParameter"),
            ("badtype", "InvalidParameterValue"),
            ("empty", "InvalidParameterValue"),
            ("toolong", "InvalidParameterValue"),
            ("group", "UnsupportedOperation"),
            ("baddelay", "InvalidParameterValue"),
        ]
        .map(|(id, code)| (id.to_owned(), code.to_owned()))
    );
}

#[test]
fn send_batch_on_fifo_sees_earlier_entries_for_deduplication() {
    let mut h = harness_with("q.fifo", json!({"Fifo": true}));
    let r = h.ok(send_batch(
        "q.fifo",
        json!([
            {"Id": "a", "Body": "x", "GroupId": "g", "DeduplicationId": "d"},
            {"Id": "b", "Body": "y", "GroupId": "g", "DeduplicationId": "d"},
            {"Id": "c", "Body": "z", "GroupId": "g"},
            {"Id": "e", "Body": "z", "GroupId": "g", "DelaySeconds": 1, "DeduplicationId": "e"},
            {"Id": "f", "Body": "w", "DeduplicationId": "f"},
        ]),
    ));
    assert_eq!(
        r["Successful"],
        json!([
            {"Id": "a", "MessageId": "m-1", "SequenceNumber": 1},
            {"Id": "b", "MessageId": "m-1", "SequenceNumber": 1},
        ])
    );
    let codes: Vec<_> = r["Failed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| (f["Id"].clone(), f["error"]["code"].clone()))
        .collect();
    assert_eq!(
        codes,
        [
            (json!("c"), json!("MissingParameter")),
            (json!("e"), json!("UnsupportedOperation")),
            (json!("f"), json!("MissingParameter")),
        ]
    );
}

fn entries_of(count: usize) -> Value {
    (0..count)
        .map(|i| json!({"Id": format!("e{i}"), "Body": "x"}))
        .collect()
}

#[test]
fn send_batch_hits_the_message_limit_in_the_middle_of_a_batch() {
    let mut h = harness_with("q", json!({}));
    for _ in 0..9_999 {
        h.ok(send_batch("q", entries_of(10)));
    }
    h.ok(send_batch("q", entries_of(5)));
    let r = h.ok(send_batch("q", entries_of(10)));
    let ok_ids: Vec<_> = r["Successful"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["Id"].clone())
        .collect();
    assert_eq!(ok_ids, ["e0", "e1", "e2", "e3", "e4"].map(Value::from));
    let failed: Vec<_> = r["Failed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| (e["Id"].clone(), e["error"]["code"].clone()))
        .collect();
    assert_eq!(
        failed,
        ["e5", "e6", "e7", "e8", "e9"].map(|id| (json!(id), json!("LimitExceeded")))
    );
}

#[test]
fn batch_shape_errors_do_nothing() {
    let mut h = harness_with("q", json!({}));
    let cases = [
        (json!([]), "EmptyBatchRequest"),
        (entries_of(11), "TooManyEntriesInBatchRequest"),
        (json!([{"Id": "a", "Body": "x"}, 5]), "InvalidBatchEntryId"),
        (
            json!([{"Id": "a", "Body": "x"}, {"Body": "x"}]),
            "InvalidBatchEntryId",
        ),
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
        (
            json!([{"Id": "a", "Body": "x"}, {"Id": "a"}, {"Body": "z"}]),
            "InvalidBatchEntryId",
        ),
    ];
    for (entries, code) in cases {
        assert_eq!(h.err(send_batch("q", entries.clone())), code, "{entries}");
        assert_eq!(h.err(delete_batch("q", entries.clone())), code, "{entries}");
    }
    let r = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(r["Counts"]["Visible"], 0);
    let r = h.ok(send_batch(
        "q",
        json!([{"Id": "a".repeat(80), "Body": "x"}]),
    ));
    assert_eq!(r["Successful"][0]["MessageId"], "m-1");
    let r = h.ok(send_batch("q", entries_of(10)));
    assert_eq!(r["Successful"].as_array().unwrap().len(), 10);
}

#[test]
fn batch_parameter_errors_follow_the_check_order() {
    let mut h = harness_with("q", json!({}));
    for command in ["SendMessageBatch", "DeleteMessageBatch"] {
        assert_eq!(h.err(json!({"Command": command})), "MissingParameter");
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
        assert_eq!(
            h.err(json!({"Command": command, "QueueName": "nope", "Entries": []})),
            "QueueDoesNotExist"
        );
    }
}

#[test]
fn delete_batch_deletes_and_reports_each_entry() {
    let mut h = harness_with("q", json!({}));
    h.ok(send_batch(
        "q",
        json!([{"Id": "a", "Body": "x"}, {"Id": "b", "Body": "y"}]),
    ));
    let handles: Vec<String> = receive(&mut h, "q", 2)
        .iter()
        .map(|m| m["ReceiptHandle"].as_str().unwrap().to_owned())
        .collect();
    let r = h.ok(delete_batch(
        "q",
        json!([
            {"Id": "1", "ReceiptHandle": handles[0]},
            {"Id": "2", "ReceiptHandle": "r-99"},
            {"Id": "3", "ReceiptHandle": handles[0]},
            {"Id": "4"},
            {"Id": "5", "ReceiptHandle": 7},
            {"Id": "6", "ReceiptHandle": handles[1]},
        ]),
    ));
    assert_eq!(r["Successful"], json!([{"Id": "1"}, {"Id": "6"}]));
    let failed: Vec<_> = r["Failed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| (f["Id"].clone(), f["error"]["code"].clone()))
        .collect();
    assert_eq!(
        failed,
        [
            (json!("2"), json!("ReceiptHandleInvalid")),
            (json!("3"), json!("ReceiptHandleExpired")),
            (json!("4"), json!("MissingParameter")),
            (json!("5"), json!("InvalidParameterValue")),
        ]
    );
    let r = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(
        r["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn delete_batch_rejects_handles_of_another_queue() {
    let mut h = harness_with("q", json!({}));
    h.ok(json!({"Command": "CreateQueue", "QueueName": "other"}));
    h.ok(json!({"Command": "SendMessage", "QueueName": "other", "Body": "x"}));
    let handle = receive(&mut h, "other", 1)[0]["ReceiptHandle"].clone();
    let r = h.ok(delete_batch(
        "q",
        json!([{"Id": "a", "ReceiptHandle": handle}]),
    ));
    assert_eq!(r["Successful"], json!([]));
    assert_eq!(r["Failed"][0]["error"]["code"], "ReceiptHandleExpired");
}
