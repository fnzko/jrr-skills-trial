mod common;

use common::Harness;
use serde_json::{Value, json};

fn create(h: &mut Harness, name: &str, attributes: Value) {
    h.ok(json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attributes}));
}

fn batch(h: &mut Harness, queue: &str, entries: Value) -> Value {
    h.ok(json!({"Command": "SendMessageBatch", "QueueName": queue, "Entries": entries}))
}

fn errors(failed: &Value) -> Vec<(String, String)> {
    failed
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            assert!(f["error"]["message"].is_string());
            assert_eq!(f.as_object().unwrap().len(), 2);
            (
                f["Id"].as_str().unwrap().to_owned(),
                f["error"]["code"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

fn pair(id: &str, code: &str) -> (String, String) {
    (id.to_owned(), code.to_owned())
}

#[test]
fn send_batch_assigns_ids_in_order() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let r = batch(&mut h, "q", json!([
        {"Id": "a", "Body": "one"},
        {"Id": "b", "Body": "two", "DelaySeconds": 10},
    ]));
    assert_eq!(
        r["Successful"],
        json!([{"Id": "a", "MessageId": "m-1"}, {"Id": "b", "MessageId": "m-2"}])
    );
    assert_eq!(r["Failed"], json!([]));
    let c = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(c["Counts"], json!({"Visible": 1, "InFlight": 0, "Delayed": 1}));
}

#[test]
fn entry_failures_do_not_stop_others_or_use_ids() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({"MaxMessageBytes": 3}));
    let r = batch(&mut h, "q", json!([
        {"Id": "a", "Body": "abcd"},
        {"Id": "b"},
        {"Id": "c", "Body": 5},
        {"Id": "d", "Body": "x", "DelaySeconds": 901},
        {"Id": "e", "Body": "x", "GroupId": "g"},
        {"Id": "f", "Body": "ok", "Extra": 1, "DelaySeconds": null},
    ]));
    assert_eq!(
        errors(&r["Failed"]),
        vec![
            pair("a", "InvalidParameterValue"),
            pair("b", "MissingParameter"),
            pair("c", "InvalidParameterValue"),
            pair("d", "InvalidParameterValue"),
            pair("e", "UnsupportedOperation"),
        ]
    );
    assert_eq!(r["Successful"], json!([{"Id": "f", "MessageId": "m-1"}]));
}

#[test]
fn fifo_entries_see_earlier_entries() {
    let mut h = Harness::new();
    create(&mut h, "f.fifo", json!({"Fifo": true}));
    let r = batch(&mut h, "f.fifo", json!([
        {"Id": "a", "Body": "x", "GroupId": "g", "DeduplicationId": "d"},
        {"Id": "b", "Body": "y", "GroupId": "g", "DeduplicationId": "d"},
        {"Id": "c", "Body": "z", "GroupId": "g", "DeduplicationId": "e"},
        {"Id": "d", "Body": "z", "GroupId": "g"},
        {"Id": "e", "Body": "z", "GroupId": "g", "DeduplicationId": "f", "DelaySeconds": 1},
    ]));
    assert_eq!(
        r["Successful"],
        json!([
            {"Id": "a", "MessageId": "m-1", "SequenceNumber": 1},
            {"Id": "b", "MessageId": "m-1", "SequenceNumber": 1},
            {"Id": "c", "MessageId": "m-2", "SequenceNumber": 2},
        ])
    );
    assert_eq!(
        errors(&r["Failed"]),
        vec![pair("d", "MissingParameter"), pair("e", "UnsupportedOperation")]
    );
}

#[test]
fn message_limit_is_seen_within_a_batch() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    for _ in 0..10_000 {
        let entries: Vec<Value> = (0..10).map(|i| json!({"Id": format!("i{i}"), "Body": "x"})).collect();
        batch(&mut h, "q", json!(entries));
    }
    let entries: Vec<Value> = (0..2).map(|i| json!({"Id": format!("i{i}"), "Body": "x"})).collect();
    let r = batch(&mut h, "q", json!(entries));
    assert_eq!(r["Successful"].as_array().unwrap().len(), 0);
    assert_eq!(errors(&r["Failed"]), vec![pair("i0", "LimitExceeded"), pair("i1", "LimitExceeded")]);
}

#[test]
fn batch_level_rules_and_check_order() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let send = |q: &str, e: Value| json!({"Command": "SendMessageBatch", "QueueName": q, "Entries": e});
    let ok = json!({"Id": "a", "Body": "x"});
    assert_eq!(h.err(json!({"Command": "SendMessageBatch", "QueueName": "q"})), "MissingParameter");
    assert_eq!(h.err(json!({"Command": "SendMessageBatch", "Entries": []})), "MissingParameter");
    assert_eq!(h.err(json!({"Command": "SendMessageBatch", "QueueName": "q", "Entries": null})), "MissingParameter");
    assert_eq!(h.err(send("q", json!({}))), "InvalidParameterValue");
    assert_eq!(h.err(send("q", json!("a"))), "InvalidParameterValue");
    assert_eq!(h.err(json!({"Command": "SendMessageBatch", "QueueName": 1, "Entries": []})), "InvalidParameterValue");
    // The lookup comes before the numbered rules.
    assert_eq!(h.err(send("nope", json!([]))), "QueueDoesNotExist");
    assert_eq!(h.err(send("q", json!([]))), "EmptyBatchRequest");
    let eleven: Vec<Value> = (0..11).map(|i| json!({"Id": format!("i{i}"), "Body": "x"})).collect();
    assert_eq!(h.err(send("q", json!(eleven))), "TooManyEntriesInBatchRequest");
    // Too many wins over bad ids.
    let mut bad: Vec<Value> = (0..11).map(|_| json!(1)).collect();
    assert_eq!(h.err(send("q", json!(bad.clone()))), "TooManyEntriesInBatchRequest");
    bad.truncate(1);
    assert_eq!(h.err(send("q", json!(bad))), "InvalidBatchEntryId");
    for id in [json!(null), json!(1), json!(""), json!("a b"), json!("é"), json!("x".repeat(81))] {
        assert_eq!(h.err(send("q", json!([{"Id": id, "Body": "x"}]))), "InvalidBatchEntryId");
    }
    assert_eq!(h.err(send("q", json!([{"Body": "x"}]))), "InvalidBatchEntryId");
    assert_eq!(h.err(send("q", json!([null]))), "InvalidBatchEntryId");
    // Invalid id anywhere beats duplicates, and a failed batch does nothing.
    assert_eq!(h.err(send("q", json!([ok, ok, {"Id": 3}]))), "InvalidBatchEntryId");
    assert_eq!(h.err(send("q", json!([ok, ok]))), "BatchEntryIdsNotDistinct");
    // Ids are case-sensitive; 80 characters are fine.
    let r = h.ok(send("q", json!([{"Id": "a", "Body": "x"}, {"Id": "A", "Body": "x"}, {"Id": "x".repeat(80), "Body": "x"}])));
    assert_eq!(r["Successful"].as_array().unwrap().len(), 3);
    assert_eq!(r["Successful"][0]["MessageId"], "m-1"); // nothing was used up before
}

#[test]
fn all_entries_failing_is_still_a_success() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let r = batch(&mut h, "q", json!([{"Id": "a"}]));
    assert_eq!(r["Successful"], json!([]));
    assert_eq!(errors(&r["Failed"]), vec![pair("a", "MissingParameter")]);
}

#[test]
fn delete_batch() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    create(&mut h, "other", json!({}));
    batch(&mut h, "q", json!([{"Id": "a", "Body": "1"}, {"Id": "b", "Body": "2"}]));
    h.ok(json!({"Command": "SendMessage", "QueueName": "other", "Body": "3"}));
    let r = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 2}));
    let (h1, h2) = (&r["Messages"][0]["ReceiptHandle"], &r["Messages"][1]["ReceiptHandle"]);
    let r = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "other"}));
    let foreign = &r["Messages"][0]["ReceiptHandle"];
    let r = h.ok(json!({"Command": "DeleteMessageBatch", "QueueName": "q", "Entries": [
        {"Id": "1", "ReceiptHandle": h1},
        {"Id": "2", "ReceiptHandle": h1},
        {"Id": "3", "ReceiptHandle": "r-99"},
        {"Id": "4"},
        {"Id": "5", "ReceiptHandle": 7},
        {"Id": "6", "ReceiptHandle": foreign},
        {"Id": "7", "ReceiptHandle": h2},
    ]}));
    assert_eq!(r["Successful"], json!([{"Id": "1"}, {"Id": "7"}]));
    assert_eq!(
        errors(&r["Failed"]),
        vec![
            pair("2", "ReceiptHandleExpired"),
            pair("3", "ReceiptHandleInvalid"),
            pair("4", "MissingParameter"),
            pair("5", "InvalidParameterValue"),
            pair("6", "ReceiptHandleExpired"),
        ]
    );
    let c = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(c["Counts"], json!({"Visible": 0, "InFlight": 0, "Delayed": 0}));
    assert_eq!(h.err(json!({"Command": "DeleteMessageBatch", "QueueName": "q", "Entries": []})), "EmptyBatchRequest");
    assert_eq!(h.err(json!({"Command": "DeleteMessageBatch", "QueueName": "q"})), "MissingParameter");
}
