mod common;

use common::Harness;
use serde_json::{Value, json};

fn create(h: &mut Harness, name: &str, attributes: Value) {
    h.ok(json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attributes}));
}

fn send_batch(queue: &str, entries: Value) -> Value {
    json!({"Command": "SendMessageBatch", "QueueName": queue, "Entries": entries})
}

fn delete_batch(queue: &str, entries: Value) -> Value {
    json!({"Command": "DeleteMessageBatch", "QueueName": queue, "Entries": entries})
}

fn error_code(entry: &Value) -> &str {
    entry["error"]["code"].as_str().unwrap()
}

#[test]
fn send_batch_assigns_ids_in_order() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let r = h.ok(send_batch(
        "q",
        json!([{"Id": "a", "Body": "x"}, {"Id": "b", "Body": "y", "DelaySeconds": 5}]),
    ));
    assert_eq!(
        r,
        json!({
            "ok": true,
            "Successful": [{"Id": "a", "MessageId": "m-1"}, {"Id": "b", "MessageId": "m-2"}],
            "Failed": [],
        })
    );
    let counts = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}))["Counts"].clone();
    assert_eq!(counts, json!({"Visible": 1, "InFlight": 0, "Delayed": 1}));
}

#[test]
fn batch_level_errors_follow_the_check_order() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let eleven: Vec<Value> = (0..11)
        .map(|i| json!({"Id": format!("e{i}"), "Body": "x"}))
        .collect();
    let cases = [
        (
            json!({"Command": "SendMessageBatch", "Entries": []}),
            "MissingParameter",
        ),
        (
            json!({"Command": "SendMessageBatch", "QueueName": "q"}),
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
        // The lookup comes before the numbered rules.
        (send_batch("nope", json!([])), "QueueDoesNotExist"),
        (send_batch("q", json!([])), "EmptyBatchRequest"),
        (
            send_batch("q", json!(eleven)),
            "TooManyEntriesInBatchRequest",
        ),
        // Too many wins over a bad id.
        (
            send_batch(
                "q",
                json!(eleven.iter().map(|_| json!(1)).collect::<Vec<_>>()),
            ),
            "TooManyEntriesInBatchRequest",
        ),
        (send_batch("q", json!([1])), "InvalidBatchEntryId"),
        (
            send_batch("q", json!([{"Body": "x"}])),
            "InvalidBatchEntryId",
        ),
        (
            send_batch("q", json!([{"Id": null, "Body": "x"}])),
            "InvalidBatchEntryId",
        ),
        (
            send_batch("q", json!([{"Id": 1, "Body": "x"}])),
            "InvalidBatchEntryId",
        ),
        (
            send_batch("q", json!([{"Id": "", "Body": "x"}])),
            "InvalidBatchEntryId",
        ),
        (
            send_batch("q", json!([{"Id": "a b", "Body": "x"}])),
            "InvalidBatchEntryId",
        ),
        (
            send_batch("q", json!([{"Id": "a".repeat(81), "Body": "x"}])),
            "InvalidBatchEntryId",
        ),
        // A bad id anywhere wins over duplicate ids.
        (
            send_batch("q", json!([{"Id": "a"}, {"Id": "a"}, {"Id": "!"}])),
            "InvalidBatchEntryId",
        ),
        (
            send_batch(
                "q",
                json!([{"Id": "a", "Body": "x"}, {"Id": "a", "Body": "y"}]),
            ),
            "BatchEntryIdsNotDistinct",
        ),
        (delete_batch("q", json!([])), "EmptyBatchRequest"),
        (
            delete_batch("q", json!([{"Id": "a"}, {"Id": "a"}])),
            "BatchEntryIdsNotDistinct",
        ),
    ];
    for (command, code) in cases {
        assert_eq!(h.err(command.clone()), code, "{command}");
    }
    let ok_ids = send_batch(
        "q",
        json!([{"Id": "a".repeat(80), "Body": "x"}, {"Id": "A_-9", "Body": "x"}]),
    );
    assert_eq!(h.ok(ok_ids)["Successful"].as_array().unwrap().len(), 2);
    // A rejected batch sent nothing: the first accepted message is the first one ever.
    let r = h.ok(send_batch("q", json!([{"Id": "z", "Body": "x"}])));
    assert_eq!(r["Successful"][0]["MessageId"], "m-3");
}

#[test]
fn failed_entries_do_not_stop_the_others() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({"MaxMessageBytes": 3}));
    let r = h.ok(send_batch(
        "q",
        json!([
            {"Id": "ok1", "Body": "abc"},
            {"Id": "nobody"},
            {"Id": "wrongtype", "Body": 5},
            {"Id": "empty", "Body": ""},
            {"Id": "toolong", "Body": "abcd"},
            {"Id": "group", "Body": "a", "GroupId": "g"},
            {"Id": "delay", "Body": "a", "DelaySeconds": 901},
            {"Id": "ok2", "Body": "a", "Extra": true, "DelaySeconds": null},
        ]),
    ));
    assert_eq!(
        r["Successful"],
        json!([{"Id": "ok1", "MessageId": "m-1"}, {"Id": "ok2", "MessageId": "m-2"}])
    );
    let failed = r["Failed"].as_array().unwrap();
    let summary: Vec<(&str, &str)> = failed
        .iter()
        .map(|f| (f["Id"].as_str().unwrap(), error_code(f)))
        .collect();
    assert_eq!(
        summary,
        [
            ("nobody", "MissingParameter"),
            ("wrongtype", "InvalidParameterValue"),
            ("empty", "InvalidParameterValue"),
            ("toolong", "InvalidParameterValue"),
            ("group", "UnsupportedOperation"),
            ("delay", "InvalidParameterValue"),
        ]
    );
    assert!(failed.iter().all(|f| f["error"]["message"].is_string()));
}

#[test]
fn fifo_entries_see_earlier_entries() {
    let mut h = Harness::new();
    create(&mut h, "q.fifo", json!({"Fifo": true}));
    let r = h.ok(send_batch(
        "q.fifo",
        json!([
            {"Id": "a", "Body": "x", "GroupId": "g", "DeduplicationId": "d"},
            {"Id": "dup", "Body": "y", "GroupId": "g", "DeduplicationId": "d"},
            {"Id": "b", "Body": "z", "GroupId": "g", "DeduplicationId": "e"},
            {"Id": "nogroup", "Body": "z", "DeduplicationId": "f"},
            {"Id": "delay", "Body": "z", "GroupId": "g", "DeduplicationId": "f", "DelaySeconds": 1},
        ]),
    ));
    assert_eq!(
        r["Successful"],
        json!([
            {"Id": "a", "MessageId": "m-1", "SequenceNumber": 1},
            {"Id": "dup", "MessageId": "m-1", "SequenceNumber": 1},
            {"Id": "b", "MessageId": "m-2", "SequenceNumber": 2},
        ])
    );
    assert_eq!(error_code(&r["Failed"][0]), "MissingParameter");
    assert_eq!(error_code(&r["Failed"][1]), "UnsupportedOperation");
}

#[test]
fn send_batch_stops_at_the_message_limit() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    for chunk in 0..10_000 {
        let entries: Vec<Value> = (0..10)
            .map(|i| json!({"Id": format!("e{i}"), "Body": "x"}))
            .collect();
        let r = h.ok(send_batch("q", json!(entries)));
        assert_eq!(r["Failed"], json!([]), "chunk {chunk}");
    }
    let r = h.ok(send_batch("q", json!([{"Id": "a", "Body": "x"}])));
    assert_eq!(r["Successful"], json!([]));
    assert_eq!(error_code(&r["Failed"][0]), "LimitExceeded");
    // Deleting one message frees exactly one slot for the next batch.
    let received = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q"}));
    let handle = received["Messages"][0]["ReceiptHandle"].clone();
    h.ok(json!({"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": handle}));
    let r = h.ok(send_batch(
        "q",
        json!([{"Id": "a", "Body": "x"}, {"Id": "b", "Body": "x"}]),
    ));
    assert_eq!(r["Successful"].as_array().unwrap().len(), 1);
    assert_eq!(error_code(&r["Failed"][0]), "LimitExceeded");
    assert_eq!(r["Failed"][0]["Id"], "b");
}

#[test]
fn delete_batch_handles_each_handle_in_order() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    create(&mut h, "other", json!({}));
    for body in ["a", "b"] {
        h.ok(json!({"Command": "SendMessage", "QueueName": "q", "Body": body}));
    }
    h.ok(json!({"Command": "SendMessage", "QueueName": "other", "Body": "c"}));
    h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 2}));
    h.ok(json!({"Command": "ReceiveMessage", "QueueName": "other"}));
    let r = h.ok(delete_batch(
        "q",
        json!([
            {"Id": "one", "ReceiptHandle": "r-1"},
            {"Id": "again", "ReceiptHandle": "r-1"},
            {"Id": "never", "ReceiptHandle": "r-99"},
            {"Id": "foreign", "ReceiptHandle": "r-3"},
            {"Id": "missing"},
            {"Id": "typed", "ReceiptHandle": 2},
            {"Id": "two", "ReceiptHandle": "r-2", "Extra": 1},
        ]),
    ));
    assert_eq!(r["Successful"], json!([{"Id": "one"}, {"Id": "two"}]));
    let summary: Vec<(&str, &str)> = r["Failed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| (f["Id"].as_str().unwrap(), error_code(f)))
        .collect();
    assert_eq!(
        summary,
        [
            ("again", "ReceiptHandleExpired"),
            ("never", "ReceiptHandleInvalid"),
            ("foreign", "ReceiptHandleExpired"),
            ("missing", "MissingParameter"),
            ("typed", "InvalidParameterValue"),
        ]
    );
    let counts = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}))["Counts"].clone();
    assert_eq!(counts, json!({"Visible": 0, "InFlight": 0, "Delayed": 0}));
}
