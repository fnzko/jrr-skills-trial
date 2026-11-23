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

fn entry_codes(response: &Value) -> Vec<(String, String)> {
    response["Failed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
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
    let r = send_batch(&mut h, "q", json!([
        {"Id": "a", "Body": "one"}, {"Id": "b", "Body": "two", "DelaySeconds": 5}, {"Id": "c", "Body": "three"}]));
    assert_eq!(r["Failed"], json!([]));
    assert_eq!(r["Successful"], json!([
        {"Id": "a", "MessageId": "m-1"}, {"Id": "b", "MessageId": "m-2"}, {"Id": "c", "MessageId": "m-3"}]));
    let got = receive(&mut h, "q", 10);
    let bodies: Vec<_> = got.iter().map(|m| m["Body"].as_str().unwrap()).collect();
    assert_eq!(bodies, ["one", "three"]); // "two" is delayed
}

#[test]
fn failing_entries_do_not_stop_the_others_or_use_ids() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({"MaxMessageBytes": 3}));
    let r = send_batch(&mut h, "q", json!([
        {"Id": "a", "Body": "ok"},
        {"Id": "b"},
        {"Id": "c", "Body": 7},
        {"Id": "d", "Body": ""},
        {"Id": "e", "Body": "toolong"},
        {"Id": "f", "Body": "x", "GroupId": "g"},
        {"Id": "g", "Body": "x", "DelaySeconds": 901},
        {"Id": "h", "Body": "yes", "Extra": [1]},
    ]));
    assert_eq!(r["Successful"], json!([{"Id": "a", "MessageId": "m-1"}, {"Id": "h", "MessageId": "m-2"}]));
    let codes: Vec<_> = entry_codes(&r).into_iter().collect();
    let expect = [("b", "MissingParameter"), ("c", "InvalidParameterValue"), ("d", "InvalidParameterValue"),
        ("e", "InvalidParameterValue"), ("f", "UnsupportedOperation"), ("g", "InvalidParameterValue")];
    assert_eq!(codes, expect.map(|(a, b)| (a.to_owned(), b.to_owned())));
    assert_eq!(receive(&mut h, "q", 10).len(), 2);
}

#[test]
fn null_members_are_absent() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let r = send_batch(&mut h, "q", json!([{"Id": "a", "Body": "x", "DelaySeconds": null, "GroupId": null}, {"Id": "b", "Body": null}]));
    assert_eq!(r["Successful"].as_array().unwrap().len(), 1);
    assert_eq!(entry_codes(&r), [("b".to_owned(), "MissingParameter".to_owned())]);
}

#[test]
fn fifo_entries_see_earlier_entries() {
    let mut h = Harness::new();
    create(&mut h, "q.fifo", json!({"Fifo": true}));
    let r = send_batch(&mut h, "q.fifo", json!([
        {"Id": "a", "Body": "x", "GroupId": "g", "DeduplicationId": "d1"},
        {"Id": "b", "Body": "y", "GroupId": "g", "DeduplicationId": "d1"},
        {"Id": "c", "Body": "z", "GroupId": "g", "DeduplicationId": "d2"},
        {"Id": "d", "Body": "z"},
        {"Id": "e", "Body": "z", "GroupId": "g", "DeduplicationId": "d3", "DelaySeconds": 1},
    ]));
    assert_eq!(r["Successful"], json!([
        {"Id": "a", "MessageId": "m-1", "SequenceNumber": 1},
        {"Id": "b", "MessageId": "m-1", "SequenceNumber": 1},
        {"Id": "c", "MessageId": "m-2", "SequenceNumber": 2},
    ]));
    assert_eq!(entry_codes(&r), [
        ("d".to_owned(), "MissingParameter".to_owned()),
        ("e".to_owned(), "UnsupportedOperation".to_owned())]);
}

#[test]
fn queue_limit_applies_entry_by_entry() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let entries = |n: usize, tag: &str| -> Value {
        (0..n).map(|i| json!({"Id": format!("{tag}{i}"), "Body": "x"})).collect()
    };
    for round in 0..10_000 {
        let r = send_batch(&mut h, "q", entries(10, &format!("r{round}_")));
        assert_eq!(r["Failed"], json!([]));
    }
    let r = send_batch(&mut h, "q", entries(2, "z"));
    assert_eq!(r["Successful"], json!([]));
    assert_eq!(entry_codes(&r).len(), 2);
    assert_eq!(entry_codes(&r)[0].1, "LimitExceeded");
}

#[test]
fn delete_batch_handles_used_handles_and_bad_entries() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    create(&mut h, "other", json!({}));
    send_batch(&mut h, "q", json!([{"Id": "a", "Body": "1"}, {"Id": "b", "Body": "2"}]));
    h.ok(json!({"Command": "SendMessage", "QueueName": "other", "Body": "3"}));
    let got = receive(&mut h, "q", 2);
    let other = receive(&mut h, "other", 1);
    let (h1, h2) = (got[0]["ReceiptHandle"].clone(), got[1]["ReceiptHandle"].clone());
    let r = delete_batch(&mut h, "q", json!([
        {"Id": "x", "ReceiptHandle": h1},
        {"Id": "y", "ReceiptHandle": h1},
        {"Id": "z", "ReceiptHandle": "r-99"},
        {"Id": "w"},
        {"Id": "v", "ReceiptHandle": 5},
        {"Id": "u", "ReceiptHandle": other[0]["ReceiptHandle"]},
        {"Id": "t", "ReceiptHandle": h2},
    ]));
    assert_eq!(r["Successful"], json!([{"Id": "x"}, {"Id": "t"}]));
    let codes = entry_codes(&r);
    let codes: Vec<_> = codes.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
    assert_eq!(codes, [("y", "ReceiptHandleExpired"), ("z", "ReceiptHandleInvalid"),
        ("w", "MissingParameter"), ("v", "InvalidParameterValue"), ("u", "ReceiptHandleExpired")]);
    let counts = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}))["Counts"].clone();
    assert_eq!(counts, json!({"Visible": 0, "InFlight": 0, "Delayed": 0}));
}

#[test]
fn batch_rules_and_their_order() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let run = |h: &mut Harness, entries: Value| {
        let send = h.err(json!({"Command": "SendMessageBatch", "QueueName": "q", "Entries": entries}));
        let delete = h.err(json!({"Command": "DeleteMessageBatch", "QueueName": "q", "Entries": entries}));
        assert_eq!(send, delete);
        send
    };
    assert_eq!(run(&mut h, json!([])), "EmptyBatchRequest");
    let eleven: Vec<Value> = (0..11).map(|i| json!({"Id": format!("i{i}"), "Body": "x"})).collect();
    assert_eq!(run(&mut h, json!(eleven)), "TooManyEntriesInBatchRequest");
    // too many wins over bad ids; bad ids win over duplicates
    let mut bad = eleven.clone();
    bad[0] = json!(1);
    assert_eq!(run(&mut h, json!(bad)), "TooManyEntriesInBatchRequest");
    for bad in [json!([1]), json!([null]), json!(["a"]), json!([[]]), json!([{"Body": "x"}]),
        json!([{"Id": null}]), json!([{"Id": 1}]), json!([{"Id": ""}]), json!([{"Id": "a b"}]),
        json!([{"Id": "a".repeat(81)}]), json!([{"Id": "a"}, {"Id": "a"}, 3])] {
        assert_eq!(run(&mut h, bad.clone()), "InvalidBatchEntryId", "{bad}");
    }
    assert_eq!(run(&mut h, json!([{"Id": "a"}, {"Id": "b"}, {"Id": "a"}])), "BatchEntryIdsNotDistinct");
    // ids are case-sensitive and up to 80 characters; a batch of failing entries still succeeds
    let r = send_batch(&mut h, "q", json!([{"Id": "A"}, {"Id": "a"}, {"Id": "a".repeat(80)}]));
    assert_eq!(r["Successful"], json!([]));
    assert_eq!(entry_codes(&r).len(), 3);
    let r = delete_batch(&mut h, "q", json!([{"Id": "A"}]));
    assert_eq!(entry_codes(&r), [("A".to_owned(), "MissingParameter".to_owned())]);
}

#[test]
fn ten_entries_are_allowed() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let ten: Vec<Value> = (0..10).map(|i| json!({"Id": format!("i{i}"), "Body": "x"})).collect();
    let r = send_batch(&mut h, "q", json!(ten));
    assert_eq!(r["Successful"].as_array().unwrap().len(), 10);
}

#[test]
fn request_level_checks_come_first() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    for command in ["SendMessageBatch", "DeleteMessageBatch"] {
        assert_eq!(h.err(json!({"Command": command, "Entries": []})), "MissingParameter");
        assert_eq!(h.err(json!({"Command": command, "QueueName": "q"})), "MissingParameter");
        assert_eq!(h.err(json!({"Command": command, "QueueName": "q", "Entries": null})), "MissingParameter");
        for bad in [json!({}), json!("a"), json!(1)] {
            assert_eq!(h.err(json!({"Command": command, "QueueName": "q", "Entries": bad})), "InvalidParameterValue");
        }
        assert_eq!(h.err(json!({"Command": command, "QueueName": 1, "Entries": []})), "InvalidParameterValue");
        // the lookup comes before the numbered rules
        assert_eq!(h.err(json!({"Command": command, "QueueName": "nope", "Entries": []})), "QueueDoesNotExist");
        assert_eq!(h.err(json!({"Command": command, "QueueName": "nope", "Entries": [1]})), "QueueDoesNotExist");
    }
}

#[test]
fn a_failed_batch_changes_nothing() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    h.err(json!({"Command": "SendMessageBatch", "QueueName": "q",
        "Entries": [{"Id": "a", "Body": "x"}, {"Id": "a", "Body": "y"}]}));
    let counts = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}))["Counts"].clone();
    assert_eq!(counts, json!({"Visible": 0, "InFlight": 0, "Delayed": 0}));
    let r = h.ok(json!({"Command": "SendMessage", "QueueName": "q", "Body": "x"}));
    assert_eq!(r["MessageId"], "m-1");
}
