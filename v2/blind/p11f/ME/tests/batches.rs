mod common;

use common::Harness;
use serde_json::{Value, json};

fn harness(attributes: Value) -> Harness {
    let mut h = Harness::new();
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": attributes}));
    h
}

fn send(h: &mut Harness, entries: Value) -> Value {
    h.ok(json!({"Command": "SendMessageBatch", "QueueName": "q", "Entries": entries}))
}

fn receive_handle(h: &mut Harness) -> String {
    let r = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q"}));
    r["Messages"][0]["ReceiptHandle"]
        .as_str()
        .unwrap()
        .to_owned()
}

#[test]
fn send_batch_accepts_entries_in_order() {
    let mut h = harness(json!({}));
    let r = send(
        &mut h,
        json!([{"Id": "a", "Body": "x"}, {"Id": "b", "Body": "y", "DelaySeconds": 5}]),
    );
    assert_eq!(
        r,
        json!({"ok": true, "Failed": [], "Successful": [{"Id": "a", "MessageId": "m-1"}, {"Id": "b", "MessageId": "m-2"}]})
    );
    let c = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}))["Counts"].clone();
    assert_eq!(c, json!({"Visible": 1, "InFlight": 0, "Delayed": 1}));
}

#[test]
fn send_batch_reports_failed_entries_without_stopping() {
    let mut h = harness(json!({"MaxMessageBytes": 3}));
    let r = send(
        &mut h,
        json!([
            {"Id": "nobody"},
            {"Id": "toolong", "Body": "abcd"},
            {"Id": "fifoonly", "Body": "x", "GroupId": "g"},
            {"Id": "baddelay", "Body": "x", "DelaySeconds": 901},
            {"Id": "ok", "Body": "x", "Extra": 1, "DelaySeconds": null}
        ]),
    );
    let codes: Vec<&Value> = r["Failed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| &f["error"]["code"])
        .collect();
    let ids: Vec<&Value> = r["Failed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| &f["Id"])
        .collect();
    assert_eq!(
        codes,
        [
            "MissingParameter",
            "InvalidParameterValue",
            "UnsupportedOperation",
            "InvalidParameterValue"
        ]
    );
    assert_eq!(ids, ["nobody", "toolong", "fifoonly", "baddelay"]);
    assert!(r["Failed"][0]["error"]["message"].is_string());
    assert_eq!(r["Successful"], json!([{"Id": "ok", "MessageId": "m-1"}]));
}

#[test]
fn send_batch_on_fifo_queue_sees_earlier_entries() {
    let mut h = Harness::new();
    h.ok(json!({"Command": "CreateQueue", "QueueName": "f.fifo", "Attributes": {"Fifo": true}}));
    let r = h.ok(
        json!({"Command": "SendMessageBatch", "QueueName": "f.fifo", "Entries": [
            {"Id": "1", "Body": "x", "GroupId": "g", "DeduplicationId": "d"},
            {"Id": "2", "Body": "y", "GroupId": "g", "DeduplicationId": "d"},
            {"Id": "3", "Body": "z", "GroupId": "g"},
            {"Id": "4", "Body": "w", "GroupId": "g", "DeduplicationId": "e"}
        ]}),
    );
    assert_eq!(
        r["Successful"],
        json!([
            {"Id": "1", "MessageId": "m-1", "SequenceNumber": 1},
            {"Id": "2", "MessageId": "m-1", "SequenceNumber": 1},
            {"Id": "4", "MessageId": "m-2", "SequenceNumber": 2}
        ])
    );
    assert_eq!(r["Failed"][0]["Id"], "3");
    assert_eq!(r["Failed"][0]["error"]["code"], "MissingParameter");
}

#[test]
fn batch_request_errors_follow_the_check_order() {
    let mut h = harness(json!({}));
    let cmd = |entries: Value| json!({"Command": "SendMessageBatch", "QueueName": "q", "Entries": entries});
    assert_eq!(
        h.err(json!({"Command": "SendMessageBatch", "QueueName": "q"})),
        "MissingParameter"
    );
    assert_eq!(
        h.err(json!({"Command": "SendMessageBatch", "Entries": []})),
        "MissingParameter"
    );
    assert_eq!(h.err(cmd(json!({}))), "InvalidParameterValue");
    assert_eq!(
        h.err(json!({"Command": "SendMessageBatch", "QueueName": "nope", "Entries": []})),
        "QueueDoesNotExist"
    );
    assert_eq!(h.err(cmd(json!([]))), "EmptyBatchRequest");
    let eleven: Vec<Value> = (0..11)
        .map(|i| json!({"Id": format!("i{i}"), "Body": "x"}))
        .collect();
    assert_eq!(h.err(cmd(json!(eleven))), "TooManyEntriesInBatchRequest");
    // Too many entries wins over bad ids.
    assert_eq!(
        h.err(cmd(json!(vec![json!(1); 11]))),
        "TooManyEntriesInBatchRequest"
    );
    for bad in [
        json!([1]),
        json!([{"Body": "x"}]),
        json!([{"Id": null, "Body": "x"}]),
        json!([{"Id": 1, "Body": "x"}]),
        json!([{"Id": "", "Body": "x"}]),
        json!([{"Id": "a b", "Body": "x"}]),
        json!([{"Id": "a".repeat(81), "Body": "x"}]),
        // Invalid ids win over duplicate ids.
        json!([{"Id": "a", "Body": "x"}, {"Id": "a", "Body": "x"}, {"Id": "!"}]),
    ] {
        assert_eq!(h.err(cmd(bad)), "InvalidBatchEntryId");
    }
    assert_eq!(
        h.err(cmd(
            json!([{"Id": "a", "Body": "x"}, {"Id": "a", "Body": "y"}])
        )),
        "BatchEntryIdsNotDistinct"
    );
    let counts = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}))["Counts"].clone();
    assert_eq!(
        counts,
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0}),
        "failed requests do nothing"
    );
}

#[test]
fn batch_accepts_ten_entries_and_80_character_ids() {
    let mut h = harness(json!({}));
    let entries: Vec<Value> = (0..10)
        .map(|i| json!({"Id": format!("{i}{}", "a".repeat(79)), "Body": "x"}))
        .collect();
    let r = send(&mut h, json!(entries));
    assert_eq!(r["Successful"].as_array().unwrap().len(), 10);
}

#[test]
fn delete_batch_deletes_and_reports_failures() {
    let mut h = harness(json!({}));
    send(
        &mut h,
        json!([{"Id": "a", "Body": "x"}, {"Id": "b", "Body": "y"}]),
    );
    let r1 = receive_handle(&mut h);
    let r2 = receive_handle(&mut h);
    let r = h.ok(
        json!({"Command": "DeleteMessageBatch", "QueueName": "q", "Entries": [
            {"Id": "1", "ReceiptHandle": r1},
            {"Id": "2", "ReceiptHandle": "r-99"},
            {"Id": "3"},
            {"Id": "4", "ReceiptHandle": r1},
            {"Id": "5", "ReceiptHandle": r2}
        ]}),
    );
    assert_eq!(r["Successful"], json!([{"Id": "1"}, {"Id": "5"}]));
    let failed: Vec<(&Value, &Value)> = r["Failed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| (&f["Id"], &f["error"]["code"]))
        .collect();
    assert_eq!(
        failed,
        [
            (&json!("2"), &json!("ReceiptHandleInvalid")),
            (&json!("3"), &json!("MissingParameter")),
            (&json!("4"), &json!("ReceiptHandleExpired"))
        ]
    );
    let c = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}))["Counts"].clone();
    assert_eq!(c, json!({"Visible": 0, "InFlight": 0, "Delayed": 0}));
}

#[test]
fn delete_batch_request_errors() {
    let mut h = harness(json!({}));
    let cmd = |entries: Value| json!({"Command": "DeleteMessageBatch", "QueueName": "q", "Entries": entries});
    assert_eq!(h.err(cmd(json!([]))), "EmptyBatchRequest");
    assert_eq!(h.err(cmd(json!(["x"]))), "InvalidBatchEntryId");
    assert_eq!(
        h.err(cmd(json!([{"Id": "a"}, {"Id": "a"}]))),
        "BatchEntryIdsNotDistinct"
    );
    assert_eq!(
        h.err(json!({"Command": "DeleteMessageBatch", "QueueName": "nope", "Entries": []})),
        "QueueDoesNotExist"
    );
}

#[test]
fn send_batch_stops_at_the_message_limit_per_entry() {
    let mut h = harness(json!({}));
    for _ in 0..10_000 {
        let entries: Vec<Value> = (0..10)
            .map(|i| json!({"Id": format!("i{i}"), "Body": "x"}))
            .collect();
        let r = send(&mut h, json!(entries));
        assert_eq!(r["Failed"], json!([]));
    }
    let r = send(&mut h, json!([{"Id": "a", "Body": "x"}]));
    assert_eq!(r["Successful"], json!([]));
    assert_eq!(r["Failed"][0]["error"]["code"], "LimitExceeded");
}
