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

fn receive(h: &mut Harness, queue: &str, max: u64) -> Vec<Value> {
    let r = h.ok(json!({"Command": "ReceiveMessage", "QueueName": queue, "MaxMessages": max}));
    r["Messages"].as_array().unwrap().clone()
}

fn error_codes(response: &Value) -> Vec<(&str, &str)> {
    response["Failed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            assert!(f["error"]["message"].is_string());
            (
                f["Id"].as_str().unwrap(),
                f["error"]["code"].as_str().unwrap(),
            )
        })
        .collect()
}

#[test]
fn send_batch_accepts_entries_in_order() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let r = send_batch(
        &mut h,
        "q",
        json!([{"Id": "a", "Body": "one"}, {"Id": "b", "Body": "two", "DelaySeconds": 5, "Extra": 1}]),
    );
    assert_eq!(
        r,
        json!({"ok": true, "Failed": [],
               "Successful": [{"Id": "a", "MessageId": "m-1"}, {"Id": "b", "MessageId": "m-2"}]})
    );
    let counts = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}))["Counts"].clone();
    assert_eq!(counts, json!({"Visible": 1, "InFlight": 0, "Delayed": 1}));
}

#[test]
fn failed_entries_do_not_stop_the_others_or_use_message_ids() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({"MaxMessageBytes": 3}));
    let r = send_batch(
        &mut h,
        "q",
        json!([
            {"Id": "a", "Body": "ok"},
            {"Id": "b"},
            {"Id": "c", "Body": 5},
            {"Id": "d", "Body": ""},
            {"Id": "e", "Body": "long!"},
            {"Id": "f", "Body": "x", "GroupId": "g"},
            {"Id": "g", "Body": "y", "DelaySeconds": 901},
            {"Id": "h", "Body": "z", "DelaySeconds": null},
        ]),
    );
    assert_eq!(
        r["Successful"],
        json!([{"Id": "a", "MessageId": "m-1"}, {"Id": "h", "MessageId": "m-2"}])
    );
    assert_eq!(
        error_codes(&r),
        vec![
            ("b", "MissingParameter"),
            ("c", "InvalidParameterValue"),
            ("d", "InvalidParameterValue"),
            ("e", "InvalidParameterValue"),
            ("f", "UnsupportedOperation"),
            ("g", "InvalidParameterValue"),
        ]
    );
}

#[test]
fn all_entries_failing_is_still_a_success() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let r = send_batch(&mut h, "q", json!([{"Id": "a"}]));
    assert_eq!(r["Successful"], json!([]));
    assert_eq!(error_codes(&r), vec![("a", "MissingParameter")]);
}

#[test]
fn fifo_batch_sees_earlier_entries_for_deduplication_and_sequence() {
    let mut h = Harness::new();
    create(&mut h, "q.fifo", json!({"Fifo": true}));
    let r = send_batch(
        &mut h,
        "q.fifo",
        json!([
            {"Id": "a", "Body": "x", "GroupId": "g", "DeduplicationId": "d1"},
            {"Id": "b", "Body": "y", "GroupId": "g", "DeduplicationId": "d1"},
            {"Id": "c", "Body": "z", "GroupId": "g", "DeduplicationId": "d2"},
            {"Id": "d", "Body": "w", "GroupId": "g"},
            {"Id": "e", "Body": "v", "DeduplicationId": "d3"},
            {"Id": "f", "Body": "u", "GroupId": "g", "DeduplicationId": "d4", "DelaySeconds": 1},
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
    assert_eq!(
        error_codes(&r),
        vec![
            ("d", "MissingParameter"),
            ("e", "MissingParameter"),
            ("f", "UnsupportedOperation"),
        ]
    );
}

#[test]
fn batch_request_rules_in_order() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let send = |entries: Value| json!({"Command": "SendMessageBatch", "QueueName": "q", "Entries": entries});
    let ok_entry = |i: u32| json!({"Id": format!("e{i}"), "Body": "x"});
    let too_many: Vec<Value> = (0..11).map(ok_entry).collect();
    let ten: Vec<Value> = (0..10).map(ok_entry).collect();
    assert_eq!(h.err(send(json!([]))), "EmptyBatchRequest");
    assert_eq!(h.err(send(json!(too_many))), "TooManyEntriesInBatchRequest");
    // Rule 2 comes before rule 3.
    let mut bad_and_many = vec![json!(5)];
    bad_and_many.extend((0..10).map(ok_entry));
    assert_eq!(
        h.err(send(json!(bad_and_many))),
        "TooManyEntriesInBatchRequest"
    );
    let bad_ids = [
        json!(5),
        json!("a"),
        json!(null),
        json!([]),
        json!({"Body": "x"}),
        json!({"Id": null, "Body": "x"}),
        json!({"Id": 1, "Body": "x"}),
        json!({"Id": "", "Body": "x"}),
        json!({"Id": "a b", "Body": "x"}),
        json!({"Id": "é", "Body": "x"}),
        json!({"Id": "a".repeat(81), "Body": "x"}),
    ];
    for entry in bad_ids {
        assert_eq!(
            h.err(send(json!([entry.clone()]))),
            "InvalidBatchEntryId",
            "{entry}"
        );
    }
    // Rule 3 comes before rule 4 and a bad entry stops the whole request.
    assert_eq!(
        h.err(send(
            json!([{"Id": "a", "Body": "x"}, {"Id": "a", "Body": "x"}, 5])
        )),
        "InvalidBatchEntryId"
    );
    assert_eq!(
        h.err(send(
            json!([{"Id": "a", "Body": "x"}, {"Id": "a", "Body": "y"}])
        )),
        "BatchEntryIdsNotDistinct"
    );
    let ok = send_batch(&mut h, "q", json!(ten));
    assert_eq!(ok["Successful"].as_array().unwrap().len(), 10);
    let ids = send_batch(
        &mut h,
        "q",
        json!([{"Id": "a".repeat(80), "Body": "x"}, {"Id": "A_-9", "Body": "x"}]),
    );
    assert_eq!(ids["Failed"], json!([]));
    // Only the 12 accepted messages exist: failed requests did nothing.
    let counts = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}))["Counts"].clone();
    assert_eq!(counts["Visible"], 12);
}

#[test]
fn batch_check_order_before_rules() {
    let mut h = Harness::new();
    for command in ["SendMessageBatch", "DeleteMessageBatch"] {
        assert_eq!(h.err(json!({"Command": command})), "MissingParameter");
        assert_eq!(
            h.err(json!({"Command": command, "QueueName": "q"})),
            "MissingParameter"
        );
        assert_eq!(
            h.err(json!({"Command": command, "QueueName": "q", "Entries": null})),
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
        // The lookup comes before the batch rules.
        assert_eq!(
            h.err(json!({"Command": command, "QueueName": "q", "Entries": []})),
            "QueueDoesNotExist"
        );
    }
}

#[test]
fn delete_batch_deletes_what_single_delete_would() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    create(&mut h, "other", json!({}));
    send_batch(
        &mut h,
        "q",
        json!([{"Id": "a", "Body": "1"}, {"Id": "b", "Body": "2"}]),
    );
    h.ok(json!({"Command": "SendMessage", "QueueName": "other", "Body": "3"}));
    let got = receive(&mut h, "q", 2);
    let foreign = receive(&mut h, "other", 1);
    let handle = |m: &Value| m["ReceiptHandle"].clone();
    let r = delete_batch(
        &mut h,
        "q",
        json!([
            {"Id": "1", "ReceiptHandle": handle(&got[0])},
            {"Id": "2", "ReceiptHandle": handle(&got[0])},
            {"Id": "3", "ReceiptHandle": "r-99"},
            {"Id": "4", "ReceiptHandle": handle(&foreign[0])},
            {"Id": "5"},
            {"Id": "6", "ReceiptHandle": 7},
            {"Id": "7", "ReceiptHandle": handle(&got[1]), "Other": true},
        ]),
    );
    assert_eq!(r["Successful"], json!([{"Id": "1"}, {"Id": "7"}]));
    assert_eq!(
        error_codes(&r),
        vec![
            ("2", "ReceiptHandleExpired"),
            ("3", "ReceiptHandleInvalid"),
            ("4", "ReceiptHandleExpired"),
            ("5", "MissingParameter"),
            ("6", "InvalidParameterValue"),
        ]
    );
    let counts = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}))["Counts"].clone();
    assert_eq!(counts, json!({"Visible": 0, "InFlight": 0, "Delayed": 0}));
}

#[test]
fn delete_batch_rules_stop_the_whole_request() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let del = |entries: Value| json!({"Command": "DeleteMessageBatch", "QueueName": "q", "Entries": entries});
    assert_eq!(h.err(del(json!([]))), "EmptyBatchRequest");
    let eleven: Vec<Value> = (0..11)
        .map(|i| json!({"Id": format!("{i}"), "ReceiptHandle": "r-1"}))
        .collect();
    assert_eq!(h.err(del(json!(eleven))), "TooManyEntriesInBatchRequest");
    assert_eq!(
        h.err(del(json!([{"ReceiptHandle": "r-1"}]))),
        "InvalidBatchEntryId"
    );
    assert_eq!(
        h.err(del(
            json!([{"Id": "a", "ReceiptHandle": "r-1"}, {"Id": "a", "ReceiptHandle": "r-2"}])
        )),
        "BatchEntryIdsNotDistinct"
    );
}

#[test]
fn batch_send_respects_the_message_limit_midway() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    // 100000 messages by batches of ten, then one more entry must fail.
    let entries: Vec<Value> = (0..10)
        .map(|i| json!({"Id": format!("e{i}"), "Body": "x"}))
        .collect();
    for _ in 0..9999 {
        h.run(json!({"Command": "SendMessageBatch", "QueueName": "q", "Entries": entries}));
    }
    let r = send_batch(
        &mut h,
        "q",
        json!([{"Id": "a", "Body": "x"}, {"Id": "b", "Body": "x"}, {"Id": "c", "Body": "x"}, {"Id": "d", "Body": "x"},
               {"Id": "e", "Body": "x"}, {"Id": "f", "Body": "x"}, {"Id": "g", "Body": "x"}, {"Id": "h", "Body": "x"},
               {"Id": "i", "Body": "x"}, {"Id": "j", "Body": "x"}]),
    );
    assert_eq!(r["Successful"].as_array().unwrap().len(), 10);
    let r = send_batch(&mut h, "q", json!([{"Id": "a", "Body": "x"}]));
    assert_eq!(error_codes(&r), vec![("a", "LimitExceeded")]);
}
