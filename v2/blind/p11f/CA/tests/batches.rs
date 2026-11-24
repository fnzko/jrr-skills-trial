mod common;

use common::Harness;
use serde_json::{Value, json};

fn harness(attributes: Value) -> Harness {
    let mut h = Harness::new();
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": attributes}));
    h
}

fn send_batch(h: &mut Harness, entries: Value) -> Value {
    h.run(json!({"Command": "SendMessageBatch", "QueueName": "q", "Entries": entries}))
}

fn receive(h: &mut Harness, max: u64) -> Vec<Value> {
    let r = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": max}));
    r["Messages"].as_array().unwrap().clone()
}

#[test]
fn send_batch_reports_each_entry_in_order() {
    let mut h = harness(json!({}));
    let r = send_batch(
        &mut h,
        json!([
            {"Id": "a", "Body": "one"},
            {"Id": "b", "Body": "", "Extra": 1},
            {"Id": "c", "Body": "three", "DelaySeconds": 5},
            {"Id": "d"},
            {"Id": "e", "Body": "x", "GroupId": "g"},
        ]),
    );
    assert_eq!(
        r["Successful"],
        json!([{"Id": "a", "MessageId": "m-1"}, {"Id": "c", "MessageId": "m-2"}])
    );
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
            ("b", "InvalidParameterValue"),
            ("d", "MissingParameter"),
            ("e", "UnsupportedOperation")
        ]
    );
    assert!(r["Failed"][0]["error"]["message"].is_string());
    let bodies: Vec<_> = receive(&mut h, 10)
        .iter()
        .map(|m| m["Body"].clone())
        .collect();
    assert_eq!(bodies, [json!("one")]);
}

#[test]
fn null_members_are_absent() {
    let mut h = harness(json!({}));
    let r = send_batch(
        &mut h,
        json!([{"Id": "a", "Body": "x", "DelaySeconds": null, "GroupId": null}]),
    );
    assert_eq!(r["Successful"], json!([{"Id": "a", "MessageId": "m-1"}]));
    let r = send_batch(&mut h, json!([{"Id": "a", "Body": null}]));
    assert_eq!(r["Failed"][0]["error"]["code"], "MissingParameter");
}

#[test]
fn request_level_errors_do_nothing() {
    let mut h = harness(json!({}));
    let eleven: Vec<Value> = (0..11)
        .map(|i| json!({"Id": format!("i{i}"), "Body": "x"}))
        .collect();
    let cases = [
        (json!([]), "EmptyBatchRequest"),
        (json!(eleven), "TooManyEntriesInBatchRequest"),
        (json!([{"Body": "x"}]), "InvalidBatchEntryId"),
        (json!([{"Id": 1, "Body": "x"}]), "InvalidBatchEntryId"),
        (json!([{"Id": null, "Body": "x"}]), "InvalidBatchEntryId"),
        (json!([{"Id": "", "Body": "x"}]), "InvalidBatchEntryId"),
        (json!([{"Id": "a b", "Body": "x"}]), "InvalidBatchEntryId"),
        (
            json!([{"Id": "a".repeat(81), "Body": "x"}]),
            "InvalidBatchEntryId",
        ),
        (json!(["a"]), "InvalidBatchEntryId"),
        (json!([{"Id": "a", "Body": "x"}, 7]), "InvalidBatchEntryId"),
        (
            json!([{"Id": "a", "Body": "x"}, {"Id": "a", "Body": "y"}]),
            "BatchEntryIdsNotDistinct",
        ),
        // an invalid id is reported before a repeated one
        (
            json!([{"Id": "a"}, {"Id": "a"}, {"Id": "$"}]),
            "InvalidBatchEntryId",
        ),
        (json!({}), "InvalidParameterValue"),
        (json!("x"), "InvalidParameterValue"),
    ];
    for (entries, code) in cases {
        assert_eq!(
            h.err(json!({"Command": "SendMessageBatch", "QueueName": "q", "Entries": entries})),
            code,
            "{entries}"
        );
    }
    assert_eq!(receive(&mut h, 10), Vec::<Value>::new());
    let r = send_batch(&mut h, json!([{"Id": "a".repeat(80), "Body": "x"}]));
    assert_eq!(r["Successful"][0]["MessageId"], "m-1");
}

#[test]
fn ten_entries_are_accepted() {
    let mut h = harness(json!({}));
    let entries: Vec<Value> = (0..10)
        .map(|i| json!({"Id": format!("i{i}"), "Body": "x"}))
        .collect();
    assert_eq!(
        send_batch(&mut h, json!(entries))["Successful"]
            .as_array()
            .unwrap()
            .len(),
        10
    );
}

#[test]
fn check_order_before_numbered_rules() {
    let mut h = Harness::new();
    let batch = |command: &str, rest: Value| {
        let mut c = json!({"Command": command});
        c.as_object_mut()
            .unwrap()
            .extend(rest.as_object().unwrap().clone());
        c
    };
    for command in ["SendMessageBatch", "DeleteMessageBatch"] {
        assert_eq!(
            h.err(batch(command, json!({"Entries": []}))),
            "MissingParameter"
        );
        assert_eq!(
            h.err(batch(command, json!({"QueueName": "nope"}))),
            "MissingParameter"
        );
        assert_eq!(
            h.err(batch(command, json!({"QueueName": "nope", "Entries": 3}))),
            "InvalidParameterValue"
        );
        assert_eq!(
            h.err(batch(command, json!({"QueueName": "nope", "Entries": []}))),
            "QueueDoesNotExist"
        );
    }
}

#[test]
fn entries_see_earlier_entries() {
    let mut h = Harness::new();
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q.fifo", "Attributes": {"Fifo": true}}));
    let r = h.ok(
        json!({"Command": "SendMessageBatch", "QueueName": "q.fifo", "Entries": [
            {"Id": "a", "Body": "x", "GroupId": "g", "DeduplicationId": "d"},
            {"Id": "b", "Body": "y", "GroupId": "g", "DeduplicationId": "d"},
            {"Id": "c", "Body": "z", "GroupId": "g", "DeduplicationId": "e"},
        ]}),
    );
    assert_eq!(
        r["Successful"],
        json!([
            {"Id": "a", "MessageId": "m-1", "SequenceNumber": 1},
            {"Id": "b", "MessageId": "m-1", "SequenceNumber": 1},
            {"Id": "c", "MessageId": "m-2", "SequenceNumber": 2},
        ])
    );
}

#[test]
fn max_message_bytes_applies_per_entry() {
    let mut h = harness(json!({"MaxMessageBytes": 2}));
    let r = send_batch(
        &mut h,
        json!([{"Id": "a", "Body": "abc"}, {"Id": "b", "Body": "ab"}]),
    );
    assert_eq!(r["Failed"][0]["Id"], "a");
    assert_eq!(r["Failed"][0]["error"]["code"], "InvalidParameterValue");
    assert_eq!(r["Successful"], json!([{"Id": "b", "MessageId": "m-1"}]));
}

#[test]
fn message_limit_is_reached_inside_a_batch() {
    let mut h = harness(json!({}));
    for _ in 0..10_000 {
        let entries: Vec<Value> = (0..10)
            .map(|i| json!({"Id": format!("i{i}"), "Body": "x"}))
            .collect();
        let r = send_batch(&mut h, json!(entries));
        assert_eq!(r["Failed"], json!([]));
    }
    let r = send_batch(&mut h, json!([{"Id": "a", "Body": "x"}]));
    assert_eq!(r["Failed"][0]["error"]["code"], "LimitExceeded");
    assert_eq!(r["Successful"], json!([]));
}

#[test]
fn delete_batch_handles_each_entry() {
    let mut h = harness(json!({"VisibilityTimeout": 100}));
    send_batch(
        &mut h,
        json!([{"Id": "a", "Body": "1"}, {"Id": "b", "Body": "2"}]),
    );
    let handles: Vec<String> = receive(&mut h, 2)
        .iter()
        .map(|m| m["ReceiptHandle"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(handles, ["r-1", "r-2"]);
    let r = h.ok(
        json!({"Command": "DeleteMessageBatch", "QueueName": "q", "Entries": [
            {"Id": "x", "ReceiptHandle": "r-1"},
            {"Id": "y", "ReceiptHandle": "r-1"},
            {"Id": "z", "ReceiptHandle": "r-99"},
            {"Id": "w"},
            {"Id": "v", "ReceiptHandle": 5},
            {"Id": "u", "ReceiptHandle": "r-2"},
        ]}),
    );
    let code = |i: usize| r["Failed"][i]["error"]["code"].clone();
    assert_eq!(r["Successful"], json!([{"Id": "x"}, {"Id": "u"}]));
    assert_eq!(r["Failed"].as_array().unwrap().len(), 4);
    assert_eq!(
        [
            ("y", code(0)),
            ("z", code(1)),
            ("w", code(2)),
            ("v", code(3))
        ]
        .map(|(id, c)| (id, c.as_str().unwrap().to_owned())),
        [
            ("y", "ReceiptHandleExpired".to_owned()),
            ("z", "ReceiptHandleInvalid".to_owned()),
            ("w", "MissingParameter".to_owned()),
            ("v", "InvalidParameterValue".to_owned()),
        ]
    );
    assert_eq!(r["Failed"][0]["Id"], "y");
    let counts = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}))["Counts"].clone();
    assert_eq!(counts, json!({"Visible": 0, "InFlight": 0, "Delayed": 0}));
}

#[test]
fn delete_batch_request_errors() {
    let mut h = harness(json!({}));
    for (entries, code) in [
        (json!([]), "EmptyBatchRequest"),
        (json!([{"ReceiptHandle": "r-1"}]), "InvalidBatchEntryId"),
        (
            json!([{"Id": "a", "ReceiptHandle": "r-1"}, {"Id": "a", "ReceiptHandle": "r-1"}]),
            "BatchEntryIdsNotDistinct",
        ),
    ] {
        assert_eq!(
            h.err(json!({"Command": "DeleteMessageBatch", "QueueName": "q", "Entries": entries})),
            code
        );
    }
}
