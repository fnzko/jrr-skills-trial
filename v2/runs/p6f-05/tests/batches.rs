mod common;

use common::Harness;
use serde_json::{Value, json};

fn setup(attributes: Value) -> Harness {
    let mut h = Harness::new();
    let name = if attributes["Fifo"] == true { "q.fifo" } else { "q" };
    h.ok(json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attributes}));
    h
}

fn send_batch(h: &mut Harness, queue: &str, entries: Value) -> Value {
    h.run(json!({"Command": "SendMessageBatch", "QueueName": queue, "Entries": entries}))
}

#[test]
fn send_batch_assigns_ids_in_order_and_reports_failures_per_entry() {
    let mut h = setup(json!({}));
    let r = send_batch(
        &mut h,
        "q",
        json!([
            {"Id": "a", "Body": "one"},
            {"Id": "b"},
            {"Id": "c", "Body": "two", "DelaySeconds": 5},
            {"Id": "d", "Body": "x", "GroupId": "g"},
            {"Id": "e", "Body": "three", "Extra": 1},
        ]),
    );
    assert_eq!(r["ok"], true);
    assert_eq!(
        r["Successful"],
        json!([{"Id": "a", "MessageId": "m-1"}, {"Id": "c", "MessageId": "m-2"}, {"Id": "e", "MessageId": "m-3"}])
    );
    let failed = r["Failed"].as_array().unwrap();
    assert_eq!(failed.len(), 2);
    assert_eq!(failed[0]["Id"], "b");
    assert_eq!(failed[0]["error"]["code"], "MissingParameter");
    assert!(failed[0]["error"]["message"].is_string());
    assert_eq!(failed[1]["Id"], "d");
    assert_eq!(failed[1]["error"]["code"], "UnsupportedOperation");
    let counts = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}))["Counts"].clone();
    assert_eq!(counts, json!({"Visible": 2, "InFlight": 0, "Delayed": 1}));
}

#[test]
fn all_entries_failing_is_still_a_success_with_both_arrays() {
    let mut h = setup(json!({"MaxMessageBytes": 1}));
    let r = send_batch(&mut h, "q", json!([{"Id": "a", "Body": "xx"}, {"Id": "b", "Body": ""}]));
    assert_eq!(r["ok"], true);
    assert_eq!(r["Successful"], json!([]));
    assert_eq!(r["Failed"][0]["error"]["code"], "InvalidParameterValue");
    assert_eq!(r["Failed"][1]["error"]["code"], "InvalidParameterValue");
}

#[test]
fn fifo_entries_see_earlier_entries_and_use_sequence_numbers() {
    let mut h = setup(json!({"Fifo": true}));
    let r = send_batch(
        &mut h,
        "q.fifo",
        json!([
            {"Id": "a", "Body": "x", "GroupId": "g", "DeduplicationId": "d1"},
            {"Id": "b", "Body": "y", "GroupId": "g", "DeduplicationId": "d1"},
            {"Id": "c", "Body": "z", "GroupId": "g", "DeduplicationId": "d2"},
            {"Id": "d", "Body": "z"},
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
    assert_eq!(r["Failed"][0]["Id"], "d");
    assert_eq!(r["Failed"][0]["error"]["code"], "MissingParameter");
}

#[test]
fn request_level_check_order() {
    let mut h = setup(json!({}));
    let run = |h: &mut Harness, c: Value| h.err(c);
    let c = |q: Value, e: Value| json!({"Command": "SendMessageBatch", "QueueName": q, "Entries": e});
    assert_eq!(run(&mut h, json!({"Command": "SendMessageBatch", "QueueName": "q"})), "MissingParameter");
    assert_eq!(run(&mut h, json!({"Command": "SendMessageBatch", "Entries": []})), "MissingParameter");
    assert_eq!(run(&mut h, c(json!("q"), json!({}))), "InvalidParameterValue");
    assert_eq!(run(&mut h, c(json!(1), json!([]))), "InvalidParameterValue");
    // The queue lookup comes before the batch rules.
    assert_eq!(run(&mut h, c(json!("nope"), json!([]))), "QueueDoesNotExist");
    assert_eq!(run(&mut h, c(json!("q"), json!([]))), "EmptyBatchRequest");
    let eleven: Vec<Value> = (0..11).map(|i| json!({"Id": format!("i{i}"), "Body": "x"})).collect();
    assert_eq!(run(&mut h, c(json!("q"), json!(eleven))), "TooManyEntriesInBatchRequest");
    // Too many entries outranks an invalid entry.
    let bad: Vec<Value> = (0..11).map(|_| json!(1)).collect();
    assert_eq!(run(&mut h, c(json!("q"), json!(bad))), "TooManyEntriesInBatchRequest");
    for entry in [json!(1), json!("a"), json!(null), json!([]), json!({"Body": "x"}), json!({"Id": null}),
        json!({"Id": 1}), json!({"Id": ""}), json!({"Id": "a b"}), json!({"Id": "x".repeat(81)})] {
        let e = json!([{"Id": "ok", "Body": "x"}, entry]);
        assert_eq!(run(&mut h, c(json!("q"), e.clone())), "InvalidBatchEntryId", "{e}");
    }
    // Invalid Id outranks duplicate Ids.
    let e = json!([{"Id": "a"}, {"Id": "a"}, 1]);
    assert_eq!(run(&mut h, c(json!("q"), e)), "InvalidBatchEntryId");
    let e = json!([{"Id": "a", "Body": "x"}, {"Id": "a", "Body": "y"}]);
    assert_eq!(run(&mut h, c(json!("q"), e)), "BatchEntryIdsNotDistinct");
    let e = json!([{"Id": "A"}, {"Id": "a"}]);
    assert_eq!(send_batch(&mut h, "q", e)["ok"], true);
    // A failed request did nothing.
    let counts = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}))["Counts"].clone();
    assert_eq!(counts, json!({"Visible": 0, "InFlight": 0, "Delayed": 0}));
}

#[test]
fn entry_id_of_80_characters_is_accepted() {
    let mut h = setup(json!({}));
    let e = json!([{"Id": "x".repeat(80), "Body": "b"}]);
    assert_eq!(send_batch(&mut h, "q", e)["Successful"][0]["MessageId"], "m-1");
}

#[test]
fn ten_entries_are_accepted() {
    let mut h = setup(json!({}));
    let entries: Vec<Value> = (0..10).map(|i| json!({"Id": format!("i{i}"), "Body": "x"})).collect();
    let r = send_batch(&mut h, "q", json!(entries));
    assert_eq!(r["Successful"].as_array().unwrap().len(), 10);
}

#[test]
fn delete_batch_handles_each_handle_once() {
    let mut h = setup(json!({}));
    send_batch(&mut h, "q", json!([{"Id": "a", "Body": "1"}, {"Id": "b", "Body": "2"}]));
    let r = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 2}));
    let h1 = r["Messages"][0]["ReceiptHandle"].clone();
    let h2 = r["Messages"][1]["ReceiptHandle"].clone();
    let r = h.run(json!({"Command": "DeleteMessageBatch", "QueueName": "q", "Entries": [
        {"Id": "x", "ReceiptHandle": h1},
        {"Id": "y", "ReceiptHandle": h1},
        {"Id": "z"},
        {"Id": "w", "ReceiptHandle": "r-99"},
        {"Id": "v", "ReceiptHandle": h2},
    ]}));
    assert_eq!(r["ok"], true);
    assert_eq!(r["Successful"], json!([{"Id": "x"}, {"Id": "v"}]));
    let codes: Vec<(&str, &str)> = r["Failed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| (f["Id"].as_str().unwrap(), f["error"]["code"].as_str().unwrap()))
        .collect();
    assert_eq!(
        codes,
        [("y", "ReceiptHandleExpired"), ("z", "MissingParameter"), ("w", "ReceiptHandleInvalid")]
    );
    let counts = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}))["Counts"].clone();
    assert_eq!(counts, json!({"Visible": 0, "InFlight": 0, "Delayed": 0}));
}

#[test]
fn delete_batch_request_errors() {
    let mut h = setup(json!({}));
    let c = |q: &str, e: Value| json!({"Command": "DeleteMessageBatch", "QueueName": q, "Entries": e});
    assert_eq!(h.err(json!({"Command": "DeleteMessageBatch", "QueueName": "q"})), "MissingParameter");
    assert_eq!(h.err(c("nope", json!([]))), "QueueDoesNotExist");
    assert_eq!(h.err(c("q", json!([]))), "EmptyBatchRequest");
    assert_eq!(h.err(c("q", json!([{"ReceiptHandle": "r-1"}]))), "InvalidBatchEntryId");
    let dup = json!([{"Id": "a", "ReceiptHandle": "r-1"}, {"Id": "a", "ReceiptHandle": "r-2"}]);
    assert_eq!(h.err(c("q", dup)), "BatchEntryIdsNotDistinct");
}

#[test]
fn batch_respects_the_queue_message_limit() {
    let mut h = setup(json!({}));
    for _ in 0..10_000 {
        let entries: Vec<Value> = (0..10).map(|i| json!({"Id": format!("i{i}"), "Body": "x"})).collect();
        let r = send_batch(&mut h, "q", json!(entries));
        assert_eq!(r["Failed"], json!([]));
    }
    let r = send_batch(&mut h, "q", json!([{"Id": "a", "Body": "x"}]));
    assert_eq!(r["Successful"], json!([]));
    assert_eq!(r["Failed"][0]["error"]["code"], "LimitExceeded");
}
