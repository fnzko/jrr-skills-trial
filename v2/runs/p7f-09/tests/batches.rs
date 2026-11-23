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

#[test]
fn send_batch_handles_entries_in_order() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let r = h.ok(send_batch(
        "q",
        json!([
            {"Id": "a", "Body": "one"},
            {"Id": "b"},
            {"Id": "c", "Body": "three", "DelaySeconds": 10},
            {"Id": "d", "Body": "four", "DelaySeconds": 901},
            {"Id": "e", "Body": "five", "GroupId": "g"},
            {"Id": "f", "Body": "six", "Extra": 1, "DelaySeconds": null},
        ]),
    ));
    assert_eq!(
        r["Successful"],
        json!([
            {"Id": "a", "MessageId": "m-1"},
            {"Id": "c", "MessageId": "m-2"},
            {"Id": "f", "MessageId": "m-3"},
        ])
    );
    let failed = r["Failed"].as_array().unwrap();
    let codes: Vec<_> = failed
        .iter()
        .map(|f| {
            (
                f["Id"].as_str().unwrap(),
                f["error"]["code"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        codes,
        [
            ("b", "MissingParameter"),
            ("d", "InvalidParameterValue"),
            ("e", "UnsupportedOperation")
        ]
    );
    assert!(failed[0]["error"]["message"].is_string());
    let a = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(
        a["Counts"],
        json!({"Visible": 2, "InFlight": 0, "Delayed": 1})
    );
}

#[test]
fn all_entries_failing_is_still_a_success() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let r = h.ok(send_batch("q", json!([{"Id": "a"}])));
    assert_eq!(r["Successful"], json!([]));
    assert_eq!(r["Failed"].as_array().unwrap().len(), 1);
}

#[test]
fn batch_entries_see_earlier_entries() {
    let mut h = Harness::new();
    create(&mut h, "q.fifo", json!({"Fifo": true}));
    let r = h.ok(send_batch(
        "q.fifo",
        json!([
            {"Id": "a", "Body": "x", "GroupId": "g", "DeduplicationId": "d"},
            {"Id": "b", "Body": "y", "GroupId": "g", "DeduplicationId": "d"},
            {"Id": "c", "Body": "z", "GroupId": "g", "DeduplicationId": "e"},
            {"Id": "d", "Body": "z", "GroupId": "g"},
        ]),
    ));
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
fn batch_respects_max_message_bytes() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({"MaxMessageBytes": 3}));
    let r = h.ok(send_batch(
        "q",
        json!([{"Id": "a", "Body": "abc"}, {"Id": "b", "Body": "abcd"}]),
    ));
    assert_eq!(r["Successful"].as_array().unwrap().len(), 1);
    assert_eq!(r["Failed"][0]["error"]["code"], "InvalidParameterValue");
}

#[test]
fn batch_request_check_order() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    for cmd in ["SendMessageBatch", "DeleteMessageBatch"] {
        let c = |q: Value, e: Value| json!({"Command": cmd, "QueueName": q, "Entries": e});
        assert_eq!(
            h.err(json!({"Command": cmd, "QueueName": "q"})),
            "MissingParameter"
        );
        assert_eq!(
            h.err(json!({"Command": cmd, "Entries": []})),
            "MissingParameter"
        );
        assert_eq!(h.err(c(json!("q"), json!("x"))), "InvalidParameterValue");
        assert_eq!(h.err(c(json!("q"), json!({}))), "InvalidParameterValue");
        assert_eq!(h.err(c(json!(1), json!([]))), "InvalidParameterValue");
        // Lookup comes before the batch rules.
        assert_eq!(h.err(c(json!("nope"), json!([]))), "QueueDoesNotExist");
        assert_eq!(h.err(c(json!("q"), json!([]))), "EmptyBatchRequest");
        let eleven: Vec<Value> = (0..11)
            .map(|i| json!({"Id": format!("i{i}"), "Body": "x", "ReceiptHandle": "r-1"}))
            .collect();
        assert_eq!(
            h.err(c(json!("q"), json!(eleven))),
            "TooManyEntriesInBatchRequest"
        );
        // Too many comes before bad ids.
        let mut bad = eleven.clone();
        bad[0] = json!(5);
        assert_eq!(
            h.err(c(json!("q"), json!(bad))),
            "TooManyEntriesInBatchRequest"
        );
        for bad_id in [
            json!(5),
            json!(null),
            json!(""),
            json!("a b"),
            json!("x".repeat(81)),
        ] {
            assert_eq!(
                h.err(c(json!("q"), json!([{"Id": bad_id, "Body": "x"}]))),
                "InvalidBatchEntryId"
            );
        }
        for bad_entry in [json!(null), json!("a"), json!([]), json!({"Body": "x"})] {
            assert_eq!(
                h.err(c(json!("q"), json!([bad_entry]))),
                "InvalidBatchEntryId"
            );
        }
        // Bad id comes before duplicate ids.
        assert_eq!(
            h.err(c(json!("q"), json!([{"Id": "a"}, {"Id": "a"}, {"Id": 1}]))),
            "InvalidBatchEntryId"
        );
        assert_eq!(
            h.err(c(
                json!("q"),
                json!([{"Id": "a"}, {"Id": "b"}, {"Id": "a"}])
            )),
            "BatchEntryIdsNotDistinct"
        );
    }
    // Ten entries and an 80-character id are fine; nothing was sent so far.
    let ten: Vec<Value> = (0..10)
        .map(|i| json!({"Id": format!("{i}").repeat(8), "Body": "x"}))
        .collect();
    let r = h.ok(send_batch("q", json!(ten)));
    assert_eq!(r["Successful"].as_array().unwrap().len(), 10);
    h.ok(send_batch(
        "q",
        json!([{"Id": "x".repeat(80), "Body": "x"}]),
    ));
}

#[test]
fn delete_batch_per_entry_errors() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    create(&mut h, "other", json!({}));
    h.ok(json!({"Command": "SendMessage", "QueueName": "q", "Body": "a"}));
    h.ok(json!({"Command": "SendMessage", "QueueName": "q", "Body": "b"}));
    h.ok(json!({"Command": "SendMessage", "QueueName": "other", "Body": "c"}));
    let r = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 2}));
    let h1 = r["Messages"][0]["ReceiptHandle"].clone();
    let h2 = r["Messages"][1]["ReceiptHandle"].clone();
    let other = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "other"}))["Messages"][0]["ReceiptHandle"].clone();
    let r = h.ok(delete_batch(
        "q",
        json!([
            {"Id": "1", "ReceiptHandle": h1},
            {"Id": "2", "ReceiptHandle": h1},
            {"Id": "3"},
            {"Id": "4", "ReceiptHandle": 7},
            {"Id": "5", "ReceiptHandle": "r-99"},
            {"Id": "6", "ReceiptHandle": other},
            {"Id": "7", "ReceiptHandle": h2},
        ]),
    ));
    assert_eq!(r["Successful"], json!([{"Id": "1"}, {"Id": "7"}]));
    let codes: Vec<_> = r["Failed"]
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
        codes,
        [
            ("2", "ReceiptHandleExpired"),
            ("3", "MissingParameter"),
            ("4", "InvalidParameterValue"),
            ("5", "ReceiptHandleInvalid"),
            ("6", "ReceiptHandleExpired"),
        ]
    );
    let a = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(
        a["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}
