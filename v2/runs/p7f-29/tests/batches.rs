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

#[test]
fn send_batch_accepts_entries_in_order() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let r = send_batch(
        &mut h,
        "q",
        json!([
            {"Id": "a", "Body": "one"}, {"Id": "b", "Body": "two", "DelaySeconds": 5}, {"Id": "c", "Body": "three"}
        ]),
    );
    assert_eq!(
        r,
        json!({"ok": true, "Failed": [], "Successful": [
            {"Id": "a", "MessageId": "m-1"}, {"Id": "b", "MessageId": "m-2"}, {"Id": "c", "MessageId": "m-3"}
        ]})
    );
    let attrs = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(
        attrs["Counts"],
        json!({"Visible": 2, "InFlight": 0, "Delayed": 1})
    );
}

#[test]
fn failed_entries_do_not_stop_the_others_or_use_ids() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({"MaxMessageBytes": 3}));
    let r = send_batch(
        &mut h,
        "q",
        json!([
            {"Id": "a", "Body": "ok"},
            {"Id": "b"},
            {"Id": "c", "Body": "toolong"},
            {"Id": "d", "Body": 5},
            {"Id": "e", "Body": "x", "GroupId": "g"},
            {"Id": "f", "Body": "fine", "DelaySeconds": 901},
            {"Id": "g", "Body": "yes"},
        ]),
    );
    assert_eq!(r["ok"], true);
    assert_eq!(
        r["Successful"],
        json!([
            {"Id": "a", "MessageId": "m-1"}, {"Id": "g", "MessageId": "m-2"}
        ])
    );
    let failed = r["Failed"].as_array().unwrap();
    let got: Vec<(&str, &str)> = failed
        .iter()
        .map(|f| {
            (
                f["Id"].as_str().unwrap(),
                f["error"]["code"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        got,
        vec![
            ("b", "MissingParameter"),
            ("c", "InvalidParameterValue"),
            ("d", "InvalidParameterValue"),
            ("e", "UnsupportedOperation"),
            ("f", "InvalidParameterValue"),
        ]
    );
    assert!(failed.iter().all(|f| f["error"]["message"].is_string()));
    let members: Vec<&str> = failed[0]
        .as_object()
        .expect("failed entries are objects")
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(members, vec!["Id", "error"]);
}

#[test]
fn entries_see_the_effects_of_earlier_entries_on_a_fifo_queue() {
    let mut h = Harness::new();
    create(&mut h, "q.fifo", json!({"Fifo": true}));
    let r = send_batch(
        &mut h,
        "q.fifo",
        json!([
            {"Id": "a", "Body": "x", "GroupId": "g", "DeduplicationId": "d1"},
            {"Id": "b", "Body": "y", "GroupId": "g", "DeduplicationId": "d1"},
            {"Id": "c", "Body": "z", "GroupId": "g", "DeduplicationId": "d2"},
            {"Id": "d", "Body": "w", "DelaySeconds": 1, "GroupId": "g", "DeduplicationId": "d3"},
            {"Id": "e", "Body": "v", "DeduplicationId": "d4"},
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
    let codes: Vec<&Value> = r["Failed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| &f["error"]["code"])
        .collect();
    assert_eq!(codes, vec!["UnsupportedOperation", "MissingParameter"]);
}

#[test]
fn the_message_limit_applies_mid_batch() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let entries: Vec<Value> = (0..10)
        .map(|i| json!({"Id": format!("e{i}"), "Body": "x"}))
        .collect();
    for _ in 0..9999 {
        assert_eq!(
            send_batch(&mut h, "q", json!(entries.clone()))["Failed"],
            json!([])
        );
    }
    let attrs = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(attrs["Counts"]["Visible"], 99_990);
    let r = send_batch(&mut h, "q", json!(entries.clone()));
    assert_eq!(r["Successful"].as_array().unwrap().len(), 10);
    let r = send_batch(&mut h, "q", json!([{"Id": "x", "Body": "x"}]));
    assert_eq!(r["Failed"][0]["error"]["code"], "LimitExceeded");
    assert_eq!(r["Successful"], json!([]));
}

#[test]
fn batch_rules_are_checked_in_order_after_lookup() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let ok_entry = json!({"Id": "a", "Body": "x"});
    let eleven: Vec<Value> = (0..11)
        .map(|i| json!({"Id": format!("e{i}"), "Body": "x"}))
        .collect();
    let cmd = |entries: Value| json!({"Command": "SendMessageBatch", "QueueName": "q", "Entries": entries});
    assert_eq!(
        h.err(json!({"Command": "SendMessageBatch", "QueueName": "q"})),
        "MissingParameter"
    );
    assert_eq!(
        h.err(json!({"Command": "SendMessageBatch", "Entries": []})),
        "MissingParameter"
    );
    assert_eq!(h.err(cmd(json!("x"))), "InvalidParameterValue");
    assert_eq!(
        h.err(json!({"Command": "SendMessageBatch", "QueueName": "none", "Entries": []})),
        "QueueDoesNotExist"
    );
    assert_eq!(h.err(cmd(json!([]))), "EmptyBatchRequest");
    assert_eq!(h.err(cmd(json!(eleven))), "TooManyEntriesInBatchRequest");
    // Too many entries beat bad ids.
    let mut bad_eleven: Vec<Value> = (0..11).map(|_| json!(1)).collect();
    assert_eq!(
        h.err(cmd(json!(bad_eleven.clone()))),
        "TooManyEntriesInBatchRequest"
    );
    bad_eleven.truncate(1);
    for bad in [
        json!(1),
        json!({"Body": "x"}),
        json!({"Id": 1}),
        json!({"Id": ""}),
        json!({"Id": "a b"}),
        json!({"Id": "x".repeat(81)}),
        json!({"Id": null, "Body": "x"}),
    ] {
        assert_eq!(
            h.err(cmd(json!([ok_entry.clone(), bad]))),
            "InvalidBatchEntryId"
        );
    }
    assert_eq!(
        h.err(cmd(json!([ok_entry.clone(), {"Id": "a", "Body": "y"}]))),
        "BatchEntryIdsNotDistinct"
    );
    // Invalid ids beat duplicates.
    assert_eq!(
        h.err(cmd(json!([ok_entry.clone(), ok_entry.clone(), 5]))),
        "InvalidBatchEntryId"
    );
    // A failed request does nothing.
    let attrs = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(attrs["Counts"]["Visible"], 0);
    // Ten entries and an 80 character id are fine.
    let ten: Vec<Value> = (0..10)
        .map(|i| json!({"Id": format!("{}{i}", "x".repeat(79)), "Body": "x"}))
        .collect();
    assert_eq!(
        h.ok(cmd(json!(ten)))["Successful"]
            .as_array()
            .unwrap()
            .len(),
        10
    );
}

#[test]
fn null_entry_members_are_absent_and_extra_members_ignored() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let r = send_batch(
        &mut h,
        "q",
        json!([
            {"Id": "a", "Body": "x", "GroupId": null, "Extra": 1, "QueueName": "other"},
            {"Id": "b", "Body": null},
        ]),
    );
    assert_eq!(r["Successful"], json!([{"Id": "a", "MessageId": "m-1"}]));
    assert_eq!(r["Failed"][0]["error"]["code"], "MissingParameter");
}

fn receive_handles(h: &mut Harness, queue: &str, n: u64) -> Vec<String> {
    let r = h.ok(json!({"Command": "ReceiveMessage", "QueueName": queue, "MaxMessages": n}));
    r["Messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["ReceiptHandle"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn delete_batch_reports_each_entry() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    create(&mut h, "other", json!({}));
    for _ in 0..3 {
        h.ok(json!({"Command": "SendMessage", "QueueName": "q", "Body": "x"}));
    }
    h.ok(json!({"Command": "SendMessage", "QueueName": "other", "Body": "x"}));
    let handles = receive_handles(&mut h, "q", 3);
    let foreign = receive_handles(&mut h, "other", 1);
    assert_eq!(handles, vec!["r-1", "r-2", "r-3"]);
    let r = delete_batch(
        &mut h,
        "q",
        json!([
            {"Id": "a", "ReceiptHandle": handles[0]},
            {"Id": "b", "ReceiptHandle": handles[0]},
            {"Id": "c", "ReceiptHandle": "r-99"},
            {"Id": "d"},
            {"Id": "e", "ReceiptHandle": 7},
            {"Id": "f", "ReceiptHandle": foreign[0]},
            {"Id": "g", "ReceiptHandle": handles[2]},
        ]),
    );
    assert_eq!(r["Successful"], json!([{"Id": "a"}, {"Id": "g"}]));
    let got: Vec<(&str, &str)> = r["Failed"]
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
        got,
        vec![
            ("b", "ReceiptHandleExpired"),
            ("c", "ReceiptHandleInvalid"),
            ("d", "MissingParameter"),
            ("e", "InvalidParameterValue"),
            ("f", "ReceiptHandleExpired"),
        ]
    );
    let attrs = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(
        attrs["Counts"],
        json!({"Visible": 0, "InFlight": 1, "Delayed": 0})
    );
}

#[test]
fn delete_batch_checks_follow_the_same_order() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let cmd = |entries: Value| json!({"Command": "DeleteMessageBatch", "QueueName": "q", "Entries": entries});
    assert_eq!(
        h.err(json!({"Command": "DeleteMessageBatch", "QueueName": "q"})),
        "MissingParameter"
    );
    assert_eq!(
        h.err(json!({"Command": "DeleteMessageBatch", "QueueName": "none", "Entries": 3})),
        "InvalidParameterValue"
    );
    assert_eq!(
        h.err(json!({"Command": "DeleteMessageBatch", "QueueName": "none", "Entries": []})),
        "QueueDoesNotExist"
    );
    assert_eq!(h.err(cmd(json!([]))), "EmptyBatchRequest");
    assert_eq!(
        h.err(cmd(json!([{"Id": "a"}, {"Id": "a"}]))),
        "BatchEntryIdsNotDistinct"
    );
    assert_eq!(h.err(cmd(json!(["a"]))), "InvalidBatchEntryId");
    let eleven: Vec<Value> = (0..11)
        .map(|i| json!({"Id": format!("e{i}"), "ReceiptHandle": "r-1"}))
        .collect();
    assert_eq!(h.err(cmd(json!(eleven))), "TooManyEntriesInBatchRequest");
}

#[test]
fn an_all_failed_batch_is_still_a_success() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let r = delete_batch(&mut h, "q", json!([{"Id": "a", "ReceiptHandle": "r-1"}]));
    assert_eq!(r["ok"], true);
    assert_eq!(r["Successful"], json!([]));
    assert_eq!(r["Failed"][0]["Id"], "a");
    assert_eq!(r["Failed"][0]["error"]["code"], "ReceiptHandleInvalid");
}
