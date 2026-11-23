mod common;

use common::Harness;
use serde_json::{Value, json};

fn create(h: &mut Harness, name: &str, attributes: Value) {
    h.ok(json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attributes}));
}

fn send_batch(h: &mut Harness, queue: &str, entries: Value) -> Value {
    h.ok(json!({"Command": "SendMessageBatch", "QueueName": queue, "Entries": entries}))
}

fn batch_error(command: &str, queue: &str, entries: Value) -> (Harness, String) {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let code = h.err(json!({"Command": command, "QueueName": queue, "Entries": entries}));
    (h, code)
}

fn failed_codes(response: &Value) -> Vec<(String, String)> {
    response["Failed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            assert_eq!(f.as_object().unwrap().len(), 2);
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
        json!([{"Id": "a", "Body": "x"}, {"Id": "b", "Body": "y"}]),
    );
    assert_eq!(
        r,
        json!({"ok": true, "Failed": [], "Successful": [
            {"Id": "a", "MessageId": "m-1"}, {"Id": "b", "MessageId": "m-2"}]})
    );
}

#[test]
fn send_batch_fifo_sees_earlier_entries_and_failures_do_not_stop_others() {
    let mut h = Harness::new();
    create(&mut h, "q.fifo", json!({"Fifo": true}));
    let r = send_batch(
        &mut h,
        "q.fifo",
        json!([
            {"Id": "1", "Body": "x", "GroupId": "g", "DeduplicationId": "d"},
            {"Id": "2", "Body": "x"},
            {"Id": "3", "Body": "y", "GroupId": "g", "DeduplicationId": "d"},
            {"Id": "4", "Body": "z", "GroupId": "g", "DeduplicationId": "e", "DelaySeconds": 1},
            {"Id": "5", "Body": "z", "GroupId": "g", "DeduplicationId": "e"},
        ]),
    );
    assert_eq!(
        r["Successful"],
        json!([
            {"Id": "1", "MessageId": "m-1", "SequenceNumber": 1},
            {"Id": "3", "MessageId": "m-1", "SequenceNumber": 1},
            {"Id": "5", "MessageId": "m-2", "SequenceNumber": 2},
        ])
    );
    assert_eq!(
        failed_codes(&r),
        vec![
            ("2".to_owned(), "MissingParameter".to_owned()),
            ("4".to_owned(), "UnsupportedOperation".to_owned()),
        ]
    );
}

#[test]
fn entries_are_checked_like_single_commands() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({"MaxMessageBytes": 2}));
    let r = send_batch(
        &mut h,
        "q",
        json!([
            {"Id": "a", "Body": "abc"},
            {"Id": "b"},
            {"Id": "c", "Body": 5},
            {"Id": "d", "Body": "ab", "DelaySeconds": 901},
            {"Id": "e", "Body": "ab", "QueueName": "other", "Extra": 1},
            {"Id": "f", "Body": "ab", "DelaySeconds": null},
        ]),
    );
    assert_eq!(
        failed_codes(&r),
        ["a", "b", "c", "d"]
            .map(|id| (
                id.to_owned(),
                if id == "b" {
                    "MissingParameter"
                } else {
                    "InvalidParameterValue"
                }
                .to_owned()
            ))
            .to_vec()
    );
    assert_eq!(
        r["Successful"],
        json!([{"Id": "e", "MessageId": "m-1"}, {"Id": "f", "MessageId": "m-2"}])
    );
}

#[test]
fn send_batch_stops_at_the_message_limit_per_entry() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    for _ in 0..10_000 {
        let entries: Vec<Value> = (0..10)
            .map(|i| json!({"Id": format!("i{i}"), "Body": "x"}))
            .collect();
        let r = send_batch(&mut h, "q", json!(entries));
        assert_eq!(r["Failed"], json!([]));
    }
    let r = send_batch(&mut h, "q", json!([{"Id": "a", "Body": "x"}]));
    assert_eq!(
        failed_codes(&r),
        vec![("a".to_owned(), "LimitExceeded".to_owned())]
    );
    assert_eq!(r["Successful"], json!([]));
}

#[test]
fn batch_level_errors_and_their_order() {
    let ids = |n: usize| -> Value {
        (0..n)
            .map(|i| json!({"Id": format!("i{i}"), "Body": "x"}))
            .collect()
    };
    for (entries, code) in [
        (json!([]), "EmptyBatchRequest"),
        (ids(11), "TooManyEntriesInBatchRequest"),
        (json!([5]), "InvalidBatchEntryId"),
        (json!([{"Body": "x"}]), "InvalidBatchEntryId"),
        (json!([{"Id": null, "Body": "x"}]), "InvalidBatchEntryId"),
        (json!([{"Id": 1, "Body": "x"}]), "InvalidBatchEntryId"),
        (json!([{"Id": "", "Body": "x"}]), "InvalidBatchEntryId"),
        (json!([{"Id": "a b", "Body": "x"}]), "InvalidBatchEntryId"),
        (json!([{"Id": "a".repeat(81)}]), "InvalidBatchEntryId"),
        // rule 3 comes before rule 4
        (json!([{"Id": "a"}, {"Id": "a"}, 1]), "InvalidBatchEntryId"),
        (
            json!([{"Id": "a"}, {"Id": "b"}, {"Id": "a"}]),
            "BatchEntryIdsNotDistinct",
        ),
    ] {
        for command in ["SendMessageBatch", "DeleteMessageBatch"] {
            let (_, got) = batch_error(command, "q", entries.clone());
            assert_eq!(got, code, "{command} {entries}");
        }
    }
    // 10 entries are fine, an Id of 80 characters is fine
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    assert_eq!(
        send_batch(&mut h, "q", ids(10))["Successful"]
            .as_array()
            .unwrap()
            .len(),
        10
    );
    let r = send_batch(&mut h, "q", json!([{"Id": "a".repeat(80), "Body": "x"}]));
    assert_eq!(r["Successful"][0]["Id"], "a".repeat(80));
}

#[test]
fn batch_parameter_checks_precede_the_lookup_and_the_rules_follow_it() {
    let mut h = Harness::new();
    for command in ["SendMessageBatch", "DeleteMessageBatch"] {
        assert_eq!(
            h.err(json!({"Command": command, "Entries": []})),
            "MissingParameter"
        );
        assert_eq!(
            h.err(json!({"Command": command, "QueueName": "nope"})),
            "MissingParameter"
        );
        assert_eq!(
            h.err(json!({"Command": command, "QueueName": "nope", "Entries": {}})),
            "InvalidParameterValue"
        );
        assert_eq!(
            h.err(json!({"Command": command, "QueueName": "nope", "Entries": []})),
            "QueueDoesNotExist"
        );
    }
}

#[test]
fn failed_batch_does_nothing() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    h.err(json!({"Command": "SendMessageBatch", "QueueName": "q",
                 "Entries": [{"Id": "a", "Body": "x"}, {"Id": "a", "Body": "y"}]}));
    let r = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(
        r["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(
        send_batch(&mut h, "q", json!([{"Id": "a", "Body": "x"}]))["Successful"][0]["MessageId"],
        "m-1"
    );
}

#[test]
fn delete_batch_handles_each_entry() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    create(&mut h, "other", json!({}));
    send_batch(
        &mut h,
        "q",
        json!([{"Id": "a", "Body": "x"}, {"Id": "b", "Body": "y"}]),
    );
    h.ok(json!({"Command": "SendMessage", "QueueName": "other", "Body": "z"}));
    let r = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 2}));
    let handles: Vec<Value> = r["Messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["ReceiptHandle"].clone())
        .collect();
    let foreign = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "other"}))["Messages"][0]["ReceiptHandle"].clone();
    let r = h.ok(
        json!({"Command": "DeleteMessageBatch", "QueueName": "q", "Entries": [
            {"Id": "1", "ReceiptHandle": handles[0]},
            {"Id": "2", "ReceiptHandle": handles[0]},
            {"Id": "3", "ReceiptHandle": "r-99"},
            {"Id": "4"},
            {"Id": "5", "ReceiptHandle": foreign},
            {"Id": "6", "ReceiptHandle": handles[1]},
        ]}),
    );
    assert_eq!(r["Successful"], json!([{"Id": "1"}, {"Id": "6"}]));
    assert_eq!(
        failed_codes(&r),
        [
            ("2", "ReceiptHandleExpired"),
            ("3", "ReceiptHandleInvalid"),
            ("4", "MissingParameter"),
            ("5", "ReceiptHandleExpired")
        ]
        .map(|(a, b)| (a.to_owned(), b.to_owned()))
        .to_vec()
    );
    let r = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(
        r["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}
