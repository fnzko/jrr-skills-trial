mod common;

use common::Harness;
use serde_json::{Value, json};

fn harness(attributes: Value) -> Harness {
    let mut h = Harness::new();
    let name = if attributes["Fifo"] == json!(true) {
        "q.fifo"
    } else {
        "q"
    };
    h.ok(json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attributes}));
    h
}

fn send_batch(h: &mut Harness, queue: &str, entries: Value) -> Value {
    h.run(json!({"Command": "SendMessageBatch", "QueueName": queue, "Entries": entries}))
}

fn delete_batch(h: &mut Harness, entries: Value) -> Value {
    h.run(json!({"Command": "DeleteMessageBatch", "QueueName": "q", "Entries": entries}))
}

fn code(response: &Value) -> &str {
    assert_eq!(response["ok"], false, "{response}");
    response["error"]["code"].as_str().unwrap()
}

#[test]
fn send_batch_sends_in_order_with_consecutive_ids() {
    let mut h = harness(json!({}));
    let r = send_batch(
        &mut h,
        "q",
        json!([{"Id": "a", "Body": "x"}, {"Id": "b", "Body": "y", "DelaySeconds": 5}]),
    );
    assert_eq!(
        r,
        json!({"ok": true, "Successful": [{"Id": "a", "MessageId": "m-1"}, {"Id": "b", "MessageId": "m-2"}], "Failed": []})
    );
    let r = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 10}));
    assert_eq!(r["Messages"].as_array().unwrap().len(), 1);
    assert_eq!(r["Messages"][0]["Body"], json!("x"));
}

#[test]
fn failed_entries_are_reported_and_do_not_stop_the_others() {
    let mut h = harness(json!({"MaxMessageBytes": 3}));
    let r = send_batch(
        &mut h,
        "q",
        json!([
            {"Id": "ok1", "Body": "abc"},
            {"Id": "nobody"},
            {"Id": "long", "Body": "abcd"},
            {"Id": "badtype", "Body": 5},
            {"Id": "badgroup", "Body": "a", "GroupId": "g"},
            {"Id": "badDelay", "Body": "a", "DelaySeconds": 901},
            {"Id": "ok2", "Body": "a", "Extra": 1},
        ]),
    );
    assert_eq!(r["ok"], true);
    assert_eq!(
        r["Successful"],
        json!([{"Id": "ok1", "MessageId": "m-1"}, {"Id": "ok2", "MessageId": "m-2"}])
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
            ("nobody", "MissingParameter"),
            ("long", "InvalidParameterValue"),
            ("badtype", "InvalidParameterValue"),
            ("badgroup", "UnsupportedOperation"),
            ("badDelay", "InvalidParameterValue"),
        ]
    );
    assert!(r["Failed"][0]["error"]["message"].is_string());
}

#[test]
fn null_members_of_an_entry_are_absent() {
    let mut h = harness(json!({}));
    let r = send_batch(
        &mut h,
        "q",
        json!([{"Id": "a", "Body": "x", "GroupId": null, "DelaySeconds": null}]),
    );
    assert_eq!(r["Successful"], json!([{"Id": "a", "MessageId": "m-1"}]));
    let r = send_batch(&mut h, "q", json!([{"Id": "a", "Body": null}]));
    assert_eq!(r["Failed"][0]["error"]["code"], json!("MissingParameter"));
}

#[test]
fn fifo_entries_see_earlier_entries() {
    let mut h = harness(json!({"Fifo": true}));
    let r = send_batch(
        &mut h,
        "q.fifo",
        json!([
            {"Id": "a", "Body": "x", "GroupId": "g", "DeduplicationId": "d"},
            {"Id": "dup", "Body": "y", "GroupId": "g", "DeduplicationId": "d"},
            {"Id": "b", "Body": "z", "GroupId": "g", "DeduplicationId": "e"},
            {"Id": "nogroup", "Body": "z", "DeduplicationId": "f"},
            {"Id": "delay", "Body": "z", "GroupId": "g", "DeduplicationId": "g", "DelaySeconds": 1},
        ]),
    );
    assert_eq!(
        r["Successful"],
        json!([
            {"Id": "a", "MessageId": "m-1", "SequenceNumber": 1},
            {"Id": "dup", "MessageId": "m-1", "SequenceNumber": 1},
            {"Id": "b", "MessageId": "m-2", "SequenceNumber": 2},
        ])
    );
    assert_eq!(r["Failed"][0]["Id"], json!("nogroup"));
    assert_eq!(r["Failed"][0]["error"]["code"], json!("MissingParameter"));
    assert_eq!(
        r["Failed"][1]["error"]["code"],
        json!("UnsupportedOperation")
    );
}

#[test]
fn max_message_bytes_applies_to_entries_before_queue_type_rules() {
    let mut h = harness(json!({"MaxMessageBytes": 2}));
    let r = send_batch(
        &mut h,
        "q",
        json!([{"Id": "a", "Body": "abc", "GroupId": "g"}]),
    );
    assert_eq!(
        r["Failed"][0]["error"]["code"],
        json!("InvalidParameterValue")
    );
}

#[test]
fn batch_wide_rules_fail_the_whole_request() {
    let mut h = harness(json!({}));
    let cases = [
        (json!([]), "EmptyBatchRequest"),
        (
            json!(
                (0..11)
                    .map(|i| json!({"Id": format!("i{i}"), "Body": "x"}))
                    .collect::<Vec<_>>()
            ),
            "TooManyEntriesInBatchRequest",
        ),
        // Too many entries wins over bad entries.
        (
            json!((0..11).map(|_| json!(5)).collect::<Vec<_>>()),
            "TooManyEntriesInBatchRequest",
        ),
        (json!([5]), "InvalidBatchEntryId"),
        (json!([[]]), "InvalidBatchEntryId"),
        (json!([{"Body": "x"}]), "InvalidBatchEntryId"),
        (json!([{"Id": null, "Body": "x"}]), "InvalidBatchEntryId"),
        (json!([{"Id": 1, "Body": "x"}]), "InvalidBatchEntryId"),
        (json!([{"Id": "", "Body": "x"}]), "InvalidBatchEntryId"),
        (json!([{"Id": "a b", "Body": "x"}]), "InvalidBatchEntryId"),
        (
            json!([{"Id": "a".repeat(81), "Body": "x"}]),
            "InvalidBatchEntryId",
        ),
        // A bad id beats a duplicate id that comes earlier in the array.
        (
            json!([{"Id": "a", "Body": "x"}, {"Id": "a", "Body": "x"}, {"Id": "!"}]),
            "InvalidBatchEntryId",
        ),
        (
            json!([{"Id": "a", "Body": "x"}, {"Id": "a", "Body": "y"}]),
            "BatchEntryIdsNotDistinct",
        ),
        (
            json!([{"Id": "a", "Body": "x"}, {"Id": "A", "Body": "y"}, {"Id": "a"}]),
            "BatchEntryIdsNotDistinct",
        ),
    ];
    for (entries, expected) in cases {
        let r = send_batch(&mut h, "q", entries.clone());
        assert_eq!(code(&r), expected, "{entries}");
    }
    // Nothing was sent by the failed requests.
    let r = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(
        r["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
    let accepted = send_batch(
        &mut h,
        "q",
        json!([{"Id": "a".repeat(80), "Body": "x"}, {"Id": "A_-9", "Body": "y"}]),
    );
    assert_eq!(accepted["Successful"][0]["MessageId"], json!("m-1"));
    let ten: Vec<Value> = (0..10)
        .map(|i| json!({"Id": format!("i{i}"), "Body": "x"}))
        .collect();
    assert_eq!(send_batch(&mut h, "q", json!(ten))["Failed"], json!([]));
}

#[test]
fn check_order_of_batch_commands() {
    let mut h = harness(json!({}));
    for command in ["SendMessageBatch", "DeleteMessageBatch"] {
        let run = |h: &mut Harness, extra: Value| {
            let mut c = json!({"Command": command});
            for (k, v) in extra.as_object().unwrap() {
                c[k] = v.clone();
            }
            h.run(c)
        };
        assert_eq!(
            code(&run(&mut h, json!({"QueueName": "q"}))),
            "MissingParameter"
        );
        assert_eq!(
            code(&run(&mut h, json!({"Entries": []}))),
            "MissingParameter"
        );
        assert_eq!(
            code(&run(&mut h, json!({"QueueName": "q", "Entries": null}))),
            "MissingParameter"
        );
        assert_eq!(
            code(&run(&mut h, json!({"QueueName": "q", "Entries": {}}))),
            "InvalidParameterValue"
        );
        assert_eq!(
            code(&run(&mut h, json!({"QueueName": 1, "Entries": "x"}))),
            "InvalidParameterValue"
        );
        assert_eq!(
            code(&run(&mut h, json!({"QueueName": "nope", "Entries": []}))),
            "QueueDoesNotExist"
        );
        assert_eq!(
            code(&run(&mut h, json!({"QueueName": "q", "Entries": []}))),
            "EmptyBatchRequest"
        );
    }
}

#[test]
fn delete_batch_handles_each_receipt_handle() {
    let mut h = harness(json!({}));
    for body in ["a", "b", "c"] {
        h.ok(json!({"Command": "SendMessage", "QueueName": "q", "Body": body}));
    }
    let r = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 3}));
    let handles: Vec<Value> = r["Messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["ReceiptHandle"].clone())
        .collect();
    let r = delete_batch(
        &mut h,
        json!([
            {"Id": "one", "ReceiptHandle": handles[0]},
            {"Id": "again", "ReceiptHandle": handles[0]},
            {"Id": "never", "ReceiptHandle": "r-99"},
            {"Id": "junk", "ReceiptHandle": "xyz"},
            {"Id": "none"},
            {"Id": "num", "ReceiptHandle": 5},
            {"Id": "two", "ReceiptHandle": handles[1]},
        ]),
    );
    assert_eq!(r["ok"], true);
    assert_eq!(r["Successful"], json!([{"Id": "one"}, {"Id": "two"}]));
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
            ("again", "ReceiptHandleExpired"),
            ("never", "ReceiptHandleInvalid"),
            ("junk", "ReceiptHandleInvalid"),
            ("none", "MissingParameter"),
            ("num", "InvalidParameterValue"),
        ]
    );
    let r = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(
        r["Counts"],
        json!({"Visible": 0, "InFlight": 1, "Delayed": 0})
    );
}

#[test]
fn delete_batch_with_all_entries_failing_is_still_a_success() {
    let mut h = harness(json!({}));
    let r = delete_batch(&mut h, json!([{"Id": "a", "ReceiptHandle": "r-1"}]));
    assert_eq!(r["ok"], true);
    assert_eq!(r["Successful"], json!([]));
    assert_eq!(r["Failed"][0]["Id"], json!("a"));
    assert_eq!(
        r["Failed"][0]["error"]["code"],
        json!("ReceiptHandleInvalid")
    );
}

#[test]
fn the_message_limit_is_seen_by_later_entries() {
    let mut h = harness(json!({}));
    // Fill 99990 slots through batches of ten.
    for batch in 0..9_999 {
        let entries: Vec<Value> = (0..10)
            .map(|i| json!({"Id": format!("e{i}"), "Body": format!("{batch}")}))
            .collect();
        assert_eq!(send_batch(&mut h, "q", json!(entries))["Failed"], json!([]));
    }
    let five: Vec<Value> = (0..5)
        .map(|i| json!({"Id": format!("e{i}"), "Body": "x"}))
        .collect();
    assert_eq!(send_batch(&mut h, "q", json!(five))["Failed"], json!([]));
    // Five slots are left for ten entries.
    let ten: Vec<Value> = (0..10)
        .map(|i| json!({"Id": format!("e{i}"), "Body": "x"}))
        .collect();
    let r = send_batch(&mut h, "q", json!(ten));
    assert_eq!(r["Successful"].as_array().unwrap().len(), 5);
    let codes: Vec<&Value> = r["Failed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| &f["error"]["code"])
        .collect();
    assert_eq!(codes, [&json!("LimitExceeded"); 5]);
    assert_eq!(r["Failed"][0]["Id"], json!("e5"));
}
