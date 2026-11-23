mod common;

use common::Harness;
use serde_json::{Value, json};

fn setup() -> Harness {
    let mut h = Harness::new();
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    h
}

fn send_batch(h: &mut Harness, entries: Value) -> Value {
    h.ok(json!({"Command": "SendMessageBatch", "QueueName": "q", "Entries": entries}))
}

fn error_code(h: &mut Harness, command: &str, entries: Value) -> String {
    h.err(json!({"Command": command, "QueueName": "q", "Entries": entries}))
}

#[test]
fn send_batch_results_in_order_with_ids() {
    let mut h = setup();
    let r = send_batch(&mut h, json!([{"Id": "a", "Body": "x"}, {"Id": "b", "Body": "y", "Extra": 1}]));
    assert_eq!(
        r["Successful"],
        json!([{"Id": "a", "MessageId": "m-1"}, {"Id": "b", "MessageId": "m-2"}])
    );
    assert_eq!(r["Failed"], json!([]));
    let r = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 10}));
    assert_eq!(r["Messages"][0]["Body"], "x");
    assert_eq!(r["Messages"][1]["Body"], "y");
}

#[test]
fn send_batch_entry_failures_do_not_stop_others_or_use_ids() {
    let mut h = setup();
    let r = send_batch(
        &mut h,
        json!([
            {"Id": "a", "Body": "x"},
            {"Id": "b"},
            {"Id": "c", "Body": 5},
            {"Id": "d", "Body": "x", "GroupId": "g"},
            {"Id": "e", "Body": "x", "DelaySeconds": 901},
            {"Id": "f", "Body": "z", "DelaySeconds": null}
        ]),
    );
    assert_eq!(
        r["Successful"],
        json!([{"Id": "a", "MessageId": "m-1"}, {"Id": "f", "MessageId": "m-2"}])
    );
    let codes: Vec<(&str, &str)> = r["Failed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| (f["Id"].as_str().unwrap(), f["error"]["code"].as_str().unwrap()))
        .collect();
    assert_eq!(
        codes,
        [
            ("b", "MissingParameter"),
            ("c", "InvalidParameterValue"),
            ("d", "UnsupportedOperation"),
            ("e", "InvalidParameterValue")
        ]
    );
    assert!(r["Failed"][0]["error"]["message"].is_string());
}

#[test]
fn send_batch_all_failed_is_still_success() {
    let mut h = setup();
    let r = send_batch(&mut h, json!([{"Id": "a"}]));
    assert_eq!(r["Successful"], json!([]));
    assert_eq!(r["Failed"].as_array().unwrap().len(), 1);
}

#[test]
fn send_batch_applies_max_message_bytes_per_entry() {
    let mut h = Harness::new();
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": {"MaxMessageBytes": 2}}));
    let r = send_batch(&mut h, json!([{"Id": "a", "Body": "ab"}, {"Id": "b", "Body": "abc"}]));
    assert_eq!(r["Successful"].as_array().unwrap().len(), 1);
    assert_eq!(r["Failed"][0]["error"]["code"], "InvalidParameterValue");
}

#[test]
fn send_batch_on_fifo_sees_earlier_entries() {
    let mut h = Harness::new();
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q.fifo", "Attributes": {"Fifo": true}}));
    let r = h.ok(json!({"Command": "SendMessageBatch", "QueueName": "q.fifo", "Entries": [
        {"Id": "a", "Body": "x", "GroupId": "g", "DeduplicationId": "d"},
        {"Id": "b", "Body": "y", "GroupId": "g", "DeduplicationId": "d"},
        {"Id": "c", "Body": "z", "GroupId": "g", "DeduplicationId": "e"},
        {"Id": "d", "Body": "z", "DeduplicationId": "f"}
    ]}));
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
fn delete_batch_reports_each_handle() {
    let mut h = setup();
    send_batch(&mut h, json!([{"Id": "a", "Body": "x"}, {"Id": "b", "Body": "y"}]));
    h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 2}));
    let r = h.ok(json!({"Command": "DeleteMessageBatch", "QueueName": "q", "Entries": [
        {"Id": "1", "ReceiptHandle": "r-1"},
        {"Id": "2", "ReceiptHandle": "r-1"},
        {"Id": "3", "ReceiptHandle": "r-99"},
        {"Id": "4"},
        {"Id": "5", "ReceiptHandle": "r-2"}
    ]}));
    assert_eq!(r["Successful"], json!([{"Id": "1"}, {"Id": "5"}]));
    let codes: Vec<&str> = r["Failed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["error"]["code"].as_str().unwrap())
        .collect();
    assert_eq!(codes, ["ReceiptHandleExpired", "ReceiptHandleInvalid", "MissingParameter"]);
    let r = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(r["Counts"], json!({"Visible": 0, "InFlight": 0, "Delayed": 0}));
}

#[test]
fn batch_rules_and_their_order() {
    for command in ["SendMessageBatch", "DeleteMessageBatch"] {
        let mut h = setup();
        assert_eq!(error_code(&mut h, command, json!([])), "EmptyBatchRequest");
        let eleven: Vec<Value> = (0..11).map(|i| json!({"Id": format!("i{i}")})).collect();
        assert_eq!(error_code(&mut h, command, json!(eleven)), "TooManyEntriesInBatchRequest");
        let ten: Vec<Value> = (0..10).map(|i| json!({"Id": format!("i{i}")})).collect();
        h.ok(json!({"Command": command, "QueueName": "q", "Entries": ten}));
        let long = "a".repeat(81);
        for bad in [json!(5), json!("x"), json!({}), json!({"Id": null}), json!({"Id": 1}),
            json!({"Id": ""}), json!({"Id": "a b"}), json!({"Id": long})] {
            assert_eq!(error_code(&mut h, command, json!([bad.clone()])), "InvalidBatchEntryId", "{bad}");
        }
        let ok80 = "a".repeat(80);
        h.ok(json!({"Command": command, "QueueName": "q", "Entries": [{"Id": ok80}]}));
        // Rule 3 is checked on every entry before rule 4.
        assert_eq!(
            error_code(&mut h, command, json!([{"Id": "a"}, {"Id": "a"}, {"Id": ""}])),
            "InvalidBatchEntryId"
        );
        assert_eq!(
            error_code(&mut h, command, json!([{"Id": "a"}, {"Id": "b"}, {"Id": "a"}])),
            "BatchEntryIdsNotDistinct"
        );
        // Ids are case-sensitive.
        let r = h.ok(json!({"Command": command, "QueueName": "q", "Entries": [{"Id": "A"}, {"Id": "a"}]}));
        assert_eq!(r["Failed"].as_array().unwrap().len(), 2);
    }
}

#[test]
fn failed_batch_does_nothing() {
    let mut h = setup();
    assert_eq!(
        error_code(&mut h, "SendMessageBatch", json!([{"Id": "a", "Body": "x"}, {"Id": "a", "Body": "y"}])),
        "BatchEntryIdsNotDistinct"
    );
    let r = h.ok(json!({"Command": "SendMessage", "QueueName": "q", "Body": "x"}));
    assert_eq!(r["MessageId"], "m-1");
}

#[test]
fn earlier_checks_win_over_batch_rules() {
    let mut h = Harness::new();
    for command in ["SendMessageBatch", "DeleteMessageBatch"] {
        assert_eq!(h.err(json!({"Command": command, "QueueName": "q"})), "MissingParameter");
        assert_eq!(h.err(json!({"Command": command, "Entries": []})), "MissingParameter");
        assert_eq!(
            h.err(json!({"Command": command, "QueueName": "q", "Entries": {}})),
            "InvalidParameterValue"
        );
        assert_eq!(
            h.err(json!({"Command": command, "QueueName": 1, "Entries": []})),
            "InvalidParameterValue"
        );
        // The queue lookup comes before the batch rules.
        assert_eq!(
            h.err(json!({"Command": command, "QueueName": "q", "Entries": []})),
            "QueueDoesNotExist"
        );
    }
}

#[test]
fn send_batch_stops_at_the_message_limit_per_entry() {
    let mut h = setup();
    h.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"MaxMessageBytes": 1}}));
    let r = send_batch(&mut h, json!([{"Id": "a", "Body": "x"}, {"Id": "b", "Body": "yy"}]));
    assert_eq!(r["Successful"].as_array().unwrap().len(), 1);
    assert_eq!(r["Failed"][0]["Id"], "b");
}
