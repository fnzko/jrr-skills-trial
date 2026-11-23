mod common;

use common::Harness;
use serde_json::{Value, json};

fn setup(attrs: Value) -> Harness {
    let mut h = Harness::new();
    let name = if attrs["Fifo"] == true { "q.fifo" } else { "q" };
    h.ok(json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attrs}));
    h
}

fn send_batch(h: &mut Harness, q: &str, entries: Value) -> Value {
    h.run(json!({"Command": "SendMessageBatch", "QueueName": q, "Entries": entries}))
}

fn code(r: &Value) -> &str {
    assert_eq!(r["ok"], false, "{r}");
    r["error"]["code"].as_str().unwrap()
}

#[test]
fn send_batch_results_in_order() {
    let mut h = setup(json!({}));
    let r = send_batch(&mut h, "q", json!([
        {"Id": "a", "Body": "one"},
        {"Id": "b", "Body": ""},
        {"Id": "c", "Body": "two", "DelaySeconds": 5},
        {"Id": "d", "Body": "x", "GroupId": "g"},
        {"Id": "e", "Body": "x", "DelaySeconds": 901},
        {"Id": "f"},
        {"Id": "g", "Body": "three", "Extra": 1},
    ]));
    assert_eq!(r["ok"], true);
    assert_eq!(r["Successful"], json!([
        {"Id": "a", "MessageId": "m-1"},
        {"Id": "c", "MessageId": "m-2"},
        {"Id": "g", "MessageId": "m-3"},
    ]));
    let failed = r["Failed"].as_array().unwrap();
    let got: Vec<(&str, &str)> = failed
        .iter()
        .map(|f| (f["Id"].as_str().unwrap(), f["error"]["code"].as_str().unwrap()))
        .collect();
    assert_eq!(got, [
        ("b", "InvalidParameterValue"),
        ("d", "UnsupportedOperation"),
        ("e", "InvalidParameterValue"),
        ("f", "MissingParameter"),
    ]);
    assert!(failed[0]["error"]["message"].is_string());
    let c = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(c["Counts"], json!({"Visible": 2, "InFlight": 0, "Delayed": 1}));
}

#[test]
fn all_entries_failing_is_still_success() {
    let mut h = setup(json!({}));
    let r = send_batch(&mut h, "q", json!([{"Id": "a"}]));
    assert_eq!(r["ok"], true);
    assert_eq!(r["Successful"], json!([]));
    assert_eq!(r["Failed"].as_array().unwrap().len(), 1);
}

#[test]
fn batch_level_checks_and_order() {
    let mut h = setup(json!({}));
    let ok = json!({"Id": "a", "Body": "x"});
    assert_eq!(code(&send_batch(&mut h, "q", json!([]))), "EmptyBatchRequest");
    assert_eq!(code(&send_batch(&mut h, "q", Value::Array(
        (0..11).map(|i| json!({"Id": format!("i{i}"), "Body": "x"})).collect()))), "TooManyEntriesInBatchRequest");
    // too many wins over bad entries
    assert_eq!(code(&send_batch(&mut h, "q", Value::Array(vec![json!(1); 11]))), "TooManyEntriesInBatchRequest");
    for bad in [json!(1), json!("a"), json!([]), json!({}), json!({"Id": 1}), json!({"Id": null}),
        json!({"Id": ""}), json!({"Id": "a b"}), json!({"Id": "x".repeat(81)})] {
        assert_eq!(code(&send_batch(&mut h, "q", json!([ok.clone(), bad.clone()]))), "InvalidBatchEntryId", "{bad}");
    }
    assert_eq!(send_batch(&mut h, "q", json!([{"Id": "x".repeat(80), "Body": "x"}]))["ok"], true);
    // invalid id beats duplicate ids
    assert_eq!(code(&send_batch(&mut h, "q", json!([ok.clone(), ok.clone(), {"Id": ""}]))), "InvalidBatchEntryId");
    assert_eq!(code(&send_batch(&mut h, "q", json!([ok.clone(), ok.clone()]))), "BatchEntryIdsNotDistinct");
    // ids are case-sensitive
    assert_eq!(send_batch(&mut h, "q", json!([{"Id": "a", "Body": "x"}, {"Id": "A", "Body": "x"}]))["ok"], true);
    // earlier checks
    assert_eq!(code(&h.run(json!({"Command": "SendMessageBatch", "Entries": []}))), "MissingParameter");
    assert_eq!(code(&h.run(json!({"Command": "SendMessageBatch", "QueueName": "q"}))), "MissingParameter");
    assert_eq!(code(&h.run(json!({"Command": "SendMessageBatch", "QueueName": "q", "Entries": {}}))), "InvalidParameterValue");
    assert_eq!(code(&send_batch(&mut h, "nope", json!([]))), "QueueDoesNotExist");
    // failed batch changed nothing: ids used so far are m-1.. only for the two successes above
    let r = send_batch(&mut h, "q", json!([{"Id": "z", "Body": "x"}]));
    assert_eq!(r["Successful"][0]["MessageId"], "m-4");
}

#[test]
fn fifo_sequence_dedup_and_limit_see_earlier_entries() {
    let mut h = setup(json!({"Fifo": true}));
    let r = send_batch(&mut h, "q.fifo", json!([
        {"Id": "a", "Body": "x", "GroupId": "g", "DeduplicationId": "d"},
        {"Id": "b", "Body": "y", "GroupId": "g", "DeduplicationId": "d"},
        {"Id": "c", "Body": "z", "GroupId": "g", "DeduplicationId": "e"},
        {"Id": "d", "Body": "z", "GroupId": "g"},
    ]));
    assert_eq!(r["Successful"], json!([
        {"Id": "a", "MessageId": "m-1", "SequenceNumber": 1},
        {"Id": "b", "MessageId": "m-1", "SequenceNumber": 1},
        {"Id": "c", "MessageId": "m-2", "SequenceNumber": 2},
    ]));
    assert_eq!(r["Failed"][0]["Id"], "d");
    assert_eq!(r["Failed"][0]["error"]["code"], "MissingParameter");
}

#[test]
fn max_message_bytes_applies_to_entries() {
    let mut h = setup(json!({"MaxMessageBytes": 2}));
    let r = send_batch(&mut h, "q", json!([{"Id": "a", "Body": "abc"}, {"Id": "b", "Body": "ab"}]));
    assert_eq!(r["Successful"], json!([{"Id": "b", "MessageId": "m-1"}]));
    assert_eq!(r["Failed"][0]["error"]["code"], "InvalidParameterValue");
}

#[test]
fn delete_batch() {
    let mut h = setup(json!({}));
    send_batch(&mut h, "q", json!([{"Id": "a", "Body": "1"}, {"Id": "b", "Body": "2"}]));
    let r = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 2}));
    let (h1, h2) = (r["Messages"][0]["ReceiptHandle"].clone(), r["Messages"][1]["ReceiptHandle"].clone());
    let r = h.run(json!({"Command": "DeleteMessageBatch", "QueueName": "q", "Entries": [
        {"Id": "1", "ReceiptHandle": h1},
        {"Id": "2", "ReceiptHandle": h1},   // already used by entry 1
        {"Id": "3", "ReceiptHandle": "r-99"},
        {"Id": "4"},
        {"Id": "5", "ReceiptHandle": h2},
    ]}));
    assert_eq!(r["ok"], true);
    assert_eq!(r["Successful"], json!([{"Id": "1"}, {"Id": "5"}]));
    let codes: Vec<(&str, &str)> = r["Failed"].as_array().unwrap().iter()
        .map(|f| (f["Id"].as_str().unwrap(), f["error"]["code"].as_str().unwrap())).collect();
    assert_eq!(codes, [("2", "ReceiptHandleExpired"), ("3", "ReceiptHandleInvalid"), ("4", "MissingParameter")]);
    let c = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(c["Counts"], json!({"Visible": 0, "InFlight": 0, "Delayed": 0}));
    let r = h.run(json!({"Command": "DeleteMessageBatch", "QueueName": "q", "Entries": []}));
    assert_eq!(code(&r), "EmptyBatchRequest");
}

#[test]
fn entry_queue_name_is_ignored() {
    let mut h = setup(json!({}));
    h.ok(json!({"Command": "CreateQueue", "QueueName": "other"}));
    let r = send_batch(&mut h, "q", json!([{"Id": "a", "Body": "x", "QueueName": "other"}]));
    assert_eq!(r["Successful"][0]["MessageId"], "m-1");
    let c = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "other"}));
    assert_eq!(c["Counts"]["Visible"], 0);
}
