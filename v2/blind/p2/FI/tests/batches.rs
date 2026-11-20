mod common;

use common::Harness;
use serde_json::{Value, json};

fn create(h: &mut Harness, name: &str, attributes: Value) {
    h.ok(json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attributes}));
}

fn send_batch(h: &mut Harness, queue: &str, entries: Value) -> Value {
    h.run(json!({"Command": "SendMessageBatch", "QueueName": queue, "Entries": entries}))
}

fn delete_batch(h: &mut Harness, queue: &str, entries: Value) -> Value {
    h.run(json!({"Command": "DeleteMessageBatch", "QueueName": queue, "Entries": entries}))
}

fn codes(response: &Value) -> Vec<(&str, &str)> {
    response["Failed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            assert!(f["error"]["message"].is_string());
            assert_eq!(f.as_object().unwrap().len(), 2);
            (
                f["Id"].as_str().unwrap(),
                f["error"]["code"].as_str().unwrap(),
            )
        })
        .collect()
}

fn counts(h: &mut Harness, queue: &str) -> Value {
    h.ok(json!({"Command": "GetQueueAttributes", "QueueName": queue}))["Counts"].clone()
}

#[test]
fn send_batch_assigns_ids_in_entry_order() {
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
    assert_eq!(
        counts(&mut h, "q"),
        json!({"Visible": 1, "InFlight": 0, "Delayed": 1})
    );
}

#[test]
fn send_batch_on_fifo_queue_returns_sequence_numbers_and_deduplicates_within_the_batch() {
    let mut h = Harness::new();
    create(&mut h, "q.fifo", json!({"Fifo": true}));
    let r = send_batch(
        &mut h,
        "q.fifo",
        json!([
            {"Id": "1", "Body": "x", "GroupId": "g", "DeduplicationId": "d"},
            {"Id": "2", "Body": "y", "GroupId": "g", "DeduplicationId": "d"},
            {"Id": "3", "Body": "z", "GroupId": "g", "DeduplicationId": "e"},
            {"Id": "4", "Body": "z"},
            {"Id": "5", "Body": "z", "GroupId": "g", "DeduplicationId": "f", "DelaySeconds": 1},
        ]),
    );
    assert_eq!(
        r["Successful"],
        json!([
            {"Id": "1", "MessageId": "m-1", "SequenceNumber": 1},
            // A duplicate succeeds with the original's identifiers.
            {"Id": "2", "MessageId": "m-1", "SequenceNumber": 1},
            {"Id": "3", "MessageId": "m-2", "SequenceNumber": 2},
        ])
    );
    assert_eq!(
        codes(&r),
        vec![("4", "MissingParameter"), ("5", "UnsupportedOperation")]
    );
}

#[test]
fn failed_entries_do_not_stop_the_others_and_use_no_message_id() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({"MaxMessageBytes": 3}));
    let r = send_batch(
        &mut h,
        "q",
        json!([
            {"Id": "a", "Body": "toolong"},
            {"Id": "b"},
            {"Id": "c", "Body": 5},
            {"Id": "d", "Body": ""},
            {"Id": "e", "Body": "ok", "DelaySeconds": 901},
            {"Id": "f", "Body": "ok", "GroupId": "g"},
            {"Id": "g", "Body": "ok", "DelaySeconds": null, "Extra": 1},
        ]),
    );
    assert_eq!(r["ok"], true);
    assert_eq!(r["Successful"], json!([{"Id": "g", "MessageId": "m-1"}]));
    assert_eq!(
        codes(&r),
        vec![
            ("a", "InvalidParameterValue"),
            ("b", "MissingParameter"),
            ("c", "InvalidParameterValue"),
            ("d", "InvalidParameterValue"),
            ("e", "InvalidParameterValue"),
            ("f", "UnsupportedOperation"),
        ]
    );
}

#[test]
fn every_entry_failing_is_still_a_success() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let r = send_batch(&mut h, "q", json!([{"Id": "a"}]));
    assert_eq!(r["ok"], true);
    assert_eq!(r["Successful"], json!([]));
    assert_eq!(codes(&r), vec![("a", "MissingParameter")]);
}

#[test]
fn entries_see_the_message_limit_effects_of_earlier_entries() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({"RetentionSeconds": 1209600}));
    // Fill the queue to one below the limit with batches, then overflow within one.
    for _ in 0..9999 {
        let entries: Vec<Value> = (0..10)
            .map(|i| json!({"Id": format!("i{i}"), "Body": "x"}))
            .collect();
        let r = send_batch(&mut h, "q", json!(entries));
        assert_eq!(r["Successful"].as_array().unwrap().len(), 10);
    }
    let entries: Vec<Value> = (0..10)
        .map(|i| json!({"Id": format!("i{i}"), "Body": "x"}))
        .collect();
    let r = send_batch(&mut h, "q", json!(entries));
    assert_eq!(r["Successful"].as_array().unwrap().len(), 10);
    let r = send_batch(&mut h, "q", json!([{"Id": "a", "Body": "x"}]));
    assert_eq!(codes(&r), vec![("a", "LimitExceeded")]);
}

#[test]
fn batch_rules_and_their_order() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let code = |h: &mut Harness, entries: Value| {
        h.err(json!({"Command": "SendMessageBatch", "QueueName": "q", "Entries": entries}))
    };
    assert_eq!(code(&mut h, json!([])), "EmptyBatchRequest");
    let eleven: Vec<Value> = (0..11)
        .map(|i| json!({"Id": format!("{i}"), "Body": "x"}))
        .collect();
    assert_eq!(code(&mut h, json!(eleven)), "TooManyEntriesInBatchRequest");
    // Rule 2 comes before rule 3, which comes before rule 4.
    let mut bad: Vec<Value> = (0..11).map(|_| json!({"Id": "same"})).collect();
    assert_eq!(
        code(&mut h, json!(bad.clone())),
        "TooManyEntriesInBatchRequest"
    );
    bad.truncate(2);
    bad.push(json!(5));
    assert_eq!(code(&mut h, json!(bad)), "InvalidBatchEntryId");
    for entry in [
        json!("x"),
        json!(null),
        json!([]),
        json!({}),
        json!({"Id": null}),
        json!({"Id": 1}),
        json!({"Id": ""}),
        json!({"Id": "a b"}),
        json!({"Id": "é"}),
        json!({"Id": "x".repeat(81)}),
    ] {
        assert_eq!(
            code(&mut h, json!([{"Id": "ok", "Body": "x"}, entry])),
            "InvalidBatchEntryId"
        );
    }
    assert_eq!(
        code(
            &mut h,
            json!([{"Id": "a", "Body": "x"}, {"Id": "a", "Body": "y"}])
        ),
        "BatchEntryIdsNotDistinct"
    );
    // Ids are case-sensitive; 80 characters is fine; 10 entries are fine.
    let ten: Vec<Value> = (0..10)
        .map(|i| json!({"Id": format!("{i}"), "Body": "x"}))
        .collect();
    assert_eq!(
        send_batch(&mut h, "q", json!(ten))["Successful"]
            .as_array()
            .unwrap()
            .len(),
        10
    );
    let r = send_batch(
        &mut h,
        "q",
        json!([{"Id": "a", "Body": "x"}, {"Id": "A", "Body": "x"}, {"Id": "x".repeat(80), "Body": "x"}]),
    );
    assert_eq!(r["Successful"].as_array().unwrap().len(), 3);
    // A request failing a batch rule does nothing.
    assert_eq!(
        counts(&mut h, "q"),
        json!({"Visible": 13, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn check_order_before_the_batch_rules() {
    let mut h = Harness::new();
    for cmd in ["SendMessageBatch", "DeleteMessageBatch"] {
        assert_eq!(
            h.err(json!({"Command": cmd, "Entries": [], "QueueName": null})),
            "MissingParameter"
        );
        assert_eq!(
            h.err(json!({"Command": cmd, "QueueName": "q"})),
            "MissingParameter"
        );
        assert_eq!(
            h.err(json!({"Command": cmd, "QueueName": "q", "Entries": null})),
            "MissingParameter"
        );
        assert_eq!(
            h.err(json!({"Command": cmd, "QueueName": 1, "Entries": []})),
            "InvalidParameterValue"
        );
        assert_eq!(
            h.err(json!({"Command": cmd, "QueueName": "q", "Entries": {}})),
            "InvalidParameterValue"
        );
        assert_eq!(
            h.err(json!({"Command": cmd, "QueueName": "q", "Entries": "x"})),
            "InvalidParameterValue"
        );
        // Lookup comes before the batch rules.
        assert_eq!(
            h.err(json!({"Command": cmd, "QueueName": "q", "Entries": []})),
            "QueueDoesNotExist"
        );
    }
}

#[test]
fn delete_batch_deletes_and_reports_per_entry() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    create(&mut h, "other", json!({}));
    for body in ["a", "b", "c"] {
        h.ok(json!({"Command": "SendMessage", "QueueName": "q", "Body": body}));
    }
    h.ok(json!({"Command": "SendMessage", "QueueName": "other", "Body": "o"}));
    let got = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 3}));
    assert_eq!(got["Messages"].as_array().unwrap().len(), 3);
    let o = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "other"}));
    assert_eq!(o["Messages"][0]["ReceiptHandle"], "r-4");

    let r = delete_batch(
        &mut h,
        "q",
        json!([
            {"Id": "1", "ReceiptHandle": "r-1"},
            {"Id": "2", "ReceiptHandle": "r-1"}, // already used by entry 1
            {"Id": "3", "ReceiptHandle": "r-4"}, // another queue
            {"Id": "4", "ReceiptHandle": "r-99"},
            {"Id": "5", "ReceiptHandle": "r-01"},
            {"Id": "6"},
            {"Id": "7", "ReceiptHandle": 3},
            {"Id": "8", "ReceiptHandle": "r-3", "Body": "ignored"},
        ]),
    );
    assert_eq!(r["ok"], true);
    assert_eq!(r["Successful"], json!([{"Id": "1"}, {"Id": "8"}]));
    assert_eq!(
        codes(&r),
        vec![
            ("2", "ReceiptHandleExpired"),
            ("3", "ReceiptHandleExpired"),
            ("4", "ReceiptHandleInvalid"),
            ("5", "ReceiptHandleInvalid"),
            ("6", "MissingParameter"),
            ("7", "InvalidParameterValue"),
        ]
    );
    assert_eq!(
        counts(&mut h, "q"),
        json!({"Visible": 0, "InFlight": 1, "Delayed": 0})
    );
}

#[test]
fn delete_batch_with_only_failures_and_expired_handles() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({"VisibilityTimeout": 5}));
    h.ok(json!({"Command": "SendMessage", "QueueName": "q", "Body": "a"}));
    h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q"}));
    h.tick(5);
    let r = delete_batch(&mut h, "q", json!([{"Id": "x", "ReceiptHandle": "r-1"}]));
    assert_eq!(r["Successful"], json!([]));
    assert_eq!(codes(&r), vec![("x", "ReceiptHandleExpired")]);
}

#[test]
fn batch_response_members_are_exactly_the_documented_ones() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let r = send_batch(&mut h, "q", json!([{"Id": "a", "Body": "x"}]));
    assert_eq!(r.as_object().unwrap().len(), 3);
    assert_eq!(r["Successful"][0].as_object().unwrap().len(), 2);
}
