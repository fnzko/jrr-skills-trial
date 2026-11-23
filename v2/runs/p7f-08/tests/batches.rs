mod common;

use common::Harness;
use serde_json::{Value, json};

fn create(h: &mut Harness, name: &str, attributes: Value) {
    h.ok(json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attributes}));
}

fn send_batch(h: &mut Harness, q: &str, entries: Value) -> Value {
    h.ok(json!({"Command": "SendMessageBatch", "QueueName": q, "Entries": entries}))
}

#[test]
fn request_checks_in_order() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    for cmd in ["SendMessageBatch", "DeleteMessageBatch"] {
        let run = |h: &mut Harness, q: Value, e: Value| {
            h.err(json!({"Command": cmd, "QueueName": q, "Entries": e}))
        };
        assert_eq!(h.err(json!({"Command": cmd, "QueueName": "q"})), "MissingParameter");
        assert_eq!(h.err(json!({"Command": cmd, "Entries": []})), "MissingParameter");
        assert_eq!(run(&mut h, json!("q"), json!("x")), "InvalidParameterValue");
        assert_eq!(run(&mut h, json!(1), json!([])), "InvalidParameterValue");
        // Lookup comes before the batch rules.
        assert_eq!(run(&mut h, json!("nope"), json!([])), "QueueDoesNotExist");
        assert_eq!(run(&mut h, json!("q"), json!([])), "EmptyBatchRequest");
        let eleven: Vec<Value> = (0..11).map(|i| json!({"Id": format!("i{i}")})).collect();
        assert_eq!(run(&mut h, json!("q"), json!(eleven)), "TooManyEntriesInBatchRequest");
        for bad in [json!(5), json!({}), json!({"Id": 1}), json!({"Id": null}), json!({"Id": ""}),
            json!({"Id": "a b"}), json!({"Id": "x".repeat(81)})] {
            assert_eq!(run(&mut h, json!("q"), json!([bad])), "InvalidBatchEntryId");
        }
        // Rule 3 beats rule 4.
        assert_eq!(
            run(&mut h, json!("q"), json!([{"Id": "a"}, {"Id": "a"}, 5])),
            "InvalidBatchEntryId"
        );
        assert_eq!(
            run(&mut h, json!("q"), json!([{"Id": "a"}, {"Id": "a"}])),
            "BatchEntryIdsNotDistinct"
        );
    }
    // Ten entries and an 80 character Id are fine.
    let ten: Vec<Value> = (0..10).map(|i| json!({"Id": format!("{i}"), "Body": "b"})).collect();
    let r = send_batch(&mut h, "q", json!(ten));
    assert_eq!(r["Successful"].as_array().unwrap().len(), 10);
    let r = send_batch(&mut h, "q", json!([{"Id": "x".repeat(80), "Body": "b"}]));
    assert_eq!(r["Successful"][0]["Id"], "x".repeat(80));
}

#[test]
fn send_batch_handles_entries_one_by_one() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({"MaxMessageBytes": 3}));
    let r = send_batch(&mut h, "q", json!([
        {"Id": "a", "Body": "one"},
        {"Id": "b", "Body": "toolong"},
        {"Id": "c"},
        {"Id": "d", "Body": "x", "GroupId": "g"},
        {"Id": "e", "Body": "two", "DelaySeconds": 901},
        {"Id": "f", "Body": "ok", "DelaySeconds": 5, "Extra": true},
    ]));
    assert_eq!(r["ok"], true);
    assert_eq!(r["Successful"], json!([{"Id": "a", "MessageId": "m-1"}, {"Id": "f", "MessageId": "m-2"}]));
    let codes: Vec<(&str, &str)> = r["Failed"].as_array().unwrap().iter()
        .map(|f| (f["Id"].as_str().unwrap(), f["error"]["code"].as_str().unwrap())).collect();
    assert_eq!(codes, [("b", "InvalidParameterValue"), ("c", "MissingParameter"),
        ("d", "UnsupportedOperation"), ("e", "InvalidParameterValue")]);
    assert!(r["Failed"][0]["error"]["message"].is_string());
    let a = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(a["Counts"], json!({"Visible": 1, "InFlight": 0, "Delayed": 1}));
}

#[test]
fn send_batch_sees_earlier_entries_on_fifo() {
    let mut h = Harness::new();
    create(&mut h, "q.fifo", json!({"Fifo": true}));
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
fn delete_batch() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    send_batch(&mut h, "q", json!([{"Id": "a", "Body": "1"}, {"Id": "b", "Body": "2"}]));
    let r = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 2}));
    let (h1, h2) = (r["Messages"][0]["ReceiptHandle"].clone(), r["Messages"][1]["ReceiptHandle"].clone());
    let r = h.ok(json!({"Command": "DeleteMessageBatch", "QueueName": "q", "Entries": [
        {"Id": "1", "ReceiptHandle": h1},
        {"Id": "2", "ReceiptHandle": h1},
        {"Id": "3"},
        {"Id": "4", "ReceiptHandle": "r-999"},
        {"Id": "5", "ReceiptHandle": 7},
        {"Id": "6", "ReceiptHandle": h2},
    ]}));
    assert_eq!(r["Successful"], json!([{"Id": "1"}, {"Id": "6"}]));
    let codes: Vec<(&str, &str)> = r["Failed"].as_array().unwrap().iter()
        .map(|f| (f["Id"].as_str().unwrap(), f["error"]["code"].as_str().unwrap())).collect();
    assert_eq!(codes, [("2", "ReceiptHandleExpired"), ("3", "MissingParameter"),
        ("4", "ReceiptHandleInvalid"), ("5", "InvalidParameterValue")]);
    let a = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(a["Counts"], json!({"Visible": 0, "InFlight": 0, "Delayed": 0}));
    // All entries failing is still a success.
    let r = h.ok(json!({"Command": "DeleteMessageBatch", "QueueName": "q", "Entries": [{"Id": "x", "ReceiptHandle": "r-1"}]}));
    assert_eq!(r["Successful"], json!([]));
    assert_eq!(r["Failed"].as_array().unwrap().len(), 1);
}

#[test]
fn batch_limit_is_seen_by_later_entries() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    for _ in 0..10_000 {
        let e: Vec<Value> = (0..10).map(|i| json!({"Id": format!("{i}"), "Body": "b"})).collect();
        send_batch(&mut h, "q", json!(e));
    }
    let r = send_batch(&mut h, "q", json!([{"Id": "a", "Body": "b"}]));
    assert_eq!(r["Failed"][0]["error"]["code"], "LimitExceeded");
}
