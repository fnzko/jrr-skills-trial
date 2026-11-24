mod common;

use common::Harness;
use serde_json::{Value, json};

fn create(h: &mut Harness, name: &str, attributes: Value) {
    h.ok(json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attributes}));
}

fn send_batch(h: &mut Harness, queue: &str, entries: Value) -> Value {
    h.ok(json!({"Command": "SendMessageBatch", "QueueName": queue, "Entries": entries}))
}

fn receive(h: &mut Harness, queue: &str, max: u64) -> Vec<Value> {
    let r = h.ok(json!({"Command": "ReceiveMessage", "QueueName": queue, "MaxMessages": max}));
    r["Messages"].as_array().unwrap().clone()
}

#[test]
fn send_batch_assigns_ids_in_entry_order() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let r = send_batch(
        &mut h,
        "q",
        json!([{"Id": "a", "Body": "x"}, {"Id": "b", "Body": "y"}]),
    );
    assert_eq!(
        r,
        json!({"ok": true, "Failed": [],
               "Successful": [{"Id": "a", "MessageId": "m-1"}, {"Id": "b", "MessageId": "m-2"}]})
    );
    let bodies: Vec<_> = receive(&mut h, "q", 10)
        .iter()
        .map(|m| m["Body"].clone())
        .collect();
    assert_eq!(bodies, [json!("x"), json!("y")]);
}

#[test]
fn failed_entries_do_not_stop_the_others_or_use_ids() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({"MaxMessageBytes": 3}));
    let r = send_batch(
        &mut h,
        "q",
        json!([
            {"Id": "1", "Body": "ok"},
            {"Id": "2", "Body": "too long"},
            {"Id": "3"},
            {"Id": "4", "Body": ""},
            {"Id": "5", "Body": "a", "DelaySeconds": 901},
            {"Id": "6", "Body": "a", "GroupId": "g"},
            {"Id": "7", "Body": "fine", "QueueName": "ignored"},
            {"Id": "8", "Body": null},
            {"Id": "9", "Body": "z"}
        ]),
    );
    assert_eq!(
        r["Successful"],
        json!([{"Id": "1", "MessageId": "m-1"}, {"Id": "9", "MessageId": "m-2"}])
    );
    let failed: Vec<(String, String)> = r["Failed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            (
                f["Id"].as_str().unwrap().to_owned(),
                f["error"]["code"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    let expected: Vec<(String, String)> = [
        ("2", "InvalidParameterValue"),
        ("3", "MissingParameter"),
        ("4", "InvalidParameterValue"),
        ("5", "InvalidParameterValue"),
        ("6", "UnsupportedOperation"),
        ("7", "InvalidParameterValue"),
        ("8", "MissingParameter"),
    ]
    .map(|(id, code)| (id.to_owned(), code.to_owned()))
    .into();
    assert_eq!(failed, expected);
    assert!(r["Failed"][0]["error"]["message"].is_string());
}

#[test]
fn entries_see_the_effects_of_earlier_entries_on_a_fifo_queue() {
    let mut h = Harness::new();
    create(&mut h, "q.fifo", json!({"Fifo": true}));
    let r = send_batch(
        &mut h,
        "q.fifo",
        json!([
            {"Id": "a", "Body": "x", "GroupId": "g", "DeduplicationId": "d"},
            {"Id": "b", "Body": "y", "GroupId": "g", "DeduplicationId": "d"},
            {"Id": "c", "Body": "z", "GroupId": "g", "DeduplicationId": "e"},
            {"Id": "d", "Body": "z", "GroupId": "g"}
        ]),
    );
    assert_eq!(
        r["Successful"],
        json!([
            {"Id": "a", "MessageId": "m-1", "SequenceNumber": 1},
            {"Id": "b", "MessageId": "m-1", "SequenceNumber": 1},
            {"Id": "c", "MessageId": "m-2", "SequenceNumber": 2}
        ])
    );
    assert_eq!(r["Failed"][0]["Id"], "d");
    assert_eq!(r["Failed"][0]["error"]["code"], "MissingParameter");
}

#[test]
fn batch_rules_and_their_order() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let cmd = |entries: Value| json!({"Command": "SendMessageBatch", "QueueName": "q", "Entries": entries});
    let ten: Vec<Value> = (0..10)
        .map(|i| json!({"Id": format!("i{i}"), "Body": "x"}))
        .collect();
    let eleven: Vec<Value> = (0..11)
        .map(|i| json!({"Id": format!("i{i}"), "Body": "x"}))
        .collect();
    assert_eq!(h.err(cmd(json!([]))), "EmptyBatchRequest");
    assert_eq!(h.err(cmd(json!(eleven))), "TooManyEntriesInBatchRequest");
    for bad in [
        json!([1]),
        json!([null]),
        json!([[]]),
        json!([{"Body": "x"}]),
        json!([{"Id": null, "Body": "x"}]),
        json!([{"Id": 1}]),
        json!([{"Id": ""}]),
        json!([{"Id": "a b"}]),
        json!([{"Id": "é"}]),
        json!([{"Id": "x".repeat(81)}]),
    ] {
        assert_eq!(h.err(cmd(bad.clone())), "InvalidBatchEntryId", "{bad}");
    }
    // A bad Id is reported before duplicates, whatever their position.
    assert_eq!(
        h.err(cmd(json!([{"Id": "a"}, {"Id": "a"}, {"Id": "!"}]))),
        "InvalidBatchEntryId"
    );
    assert_eq!(
        h.err(cmd(
            json!([{"Id": "a", "Body": "x"}, {"Id": "a", "Body": "y"}])
        )),
        "BatchEntryIdsNotDistinct"
    );
    // Ids are case-sensitive and 80 characters are allowed.
    let r = h.ok(cmd(json!([{"Id": "a", "Body": "x"}, {"Id": "A", "Body": "x"}, {"Id": "x".repeat(80), "Body": "x"}])));
    assert_eq!(r["Successful"].as_array().unwrap().len(), 3);
    let r = h.ok(cmd(json!(ten)));
    assert_eq!(r["Successful"].as_array().unwrap().len(), 10);
    // Rejected requests do nothing: 13 messages, ids m-1 to m-13.
    assert_eq!(receive(&mut h, "q", 10).len(), 10);
}

#[test]
fn batch_check_order_before_the_numbered_rules() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let mut code = |cmd: Value| h.err(cmd);
    assert_eq!(
        code(json!({"Command": "SendMessageBatch", "Entries": []})),
        "MissingParameter"
    );
    assert_eq!(
        code(json!({"Command": "SendMessageBatch", "QueueName": "q"})),
        "MissingParameter"
    );
    assert_eq!(
        code(json!({"Command": "SendMessageBatch", "QueueName": "q", "Entries": {}})),
        "InvalidParameterValue"
    );
    assert_eq!(
        code(json!({"Command": "SendMessageBatch", "QueueName": 1, "Entries": []})),
        "InvalidParameterValue"
    );
    assert_eq!(
        code(json!({"Command": "SendMessageBatch", "QueueName": "nope", "Entries": []})),
        "QueueDoesNotExist"
    );
    assert_eq!(
        code(json!({"Command": "DeleteMessageBatch", "QueueName": "nope", "Entries": 5})),
        "InvalidParameterValue"
    );
    assert_eq!(
        code(json!({"Command": "DeleteMessageBatch", "QueueName": "q", "Entries": []})),
        "EmptyBatchRequest"
    );
}

#[test]
fn delete_batch_reports_each_handle() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    send_batch(
        &mut h,
        "q",
        json!([{"Id": "a", "Body": "x"}, {"Id": "b", "Body": "y"}]),
    );
    let handles: Vec<Value> = receive(&mut h, "q", 2)
        .iter()
        .map(|m| m["ReceiptHandle"].clone())
        .collect();
    let r = h.ok(
        json!({"Command": "DeleteMessageBatch", "QueueName": "q", "Entries": [
            {"Id": "1", "ReceiptHandle": handles[0]},
            {"Id": "2", "ReceiptHandle": "r-99"},
            {"Id": "3", "ReceiptHandle": handles[0]},
            {"Id": "4"},
            {"Id": "5", "ReceiptHandle": handles[1]}
        ]}),
    );
    assert_eq!(r["Successful"], json!([{"Id": "1"}, {"Id": "5"}]));
    let failed: Vec<(&str, &str)> = r["Failed"]
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
        failed,
        [
            ("2", "ReceiptHandleInvalid"),
            ("3", "ReceiptHandleExpired"),
            ("4", "MissingParameter")
        ]
    );
    let attrs = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(
        attrs["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn whole_batch_failing_is_still_a_success_response() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let r = h.ok(json!({"Command": "DeleteMessageBatch", "QueueName": "q",
                        "Entries": [{"Id": "a", "ReceiptHandle": "bad"}]}));
    assert_eq!(r["Successful"], json!([]));
    assert_eq!(r["Failed"][0]["error"]["code"], "ReceiptHandleInvalid");
}
