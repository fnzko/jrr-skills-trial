mod common;

use common::{error_code, run};
use serde_json::{Value, json};

fn create(name: &str, attributes: Value) -> Value {
    json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attributes})
}

fn send(queue: &str, body: &str) -> Value {
    json!({"Command": "SendMessage", "QueueName": queue, "Body": body})
}

fn receive(queue: &str) -> Value {
    json!({"Command": "ReceiveMessage", "QueueName": queue})
}

fn tick(seconds: u32) -> Value {
    json!({"Command": "Tick", "Seconds": seconds})
}

fn counts(queue: &str) -> Value {
    json!({"Command": "GetQueueAttributes", "QueueName": queue})
}

#[test]
fn send_receive_roundtrip_and_identifiers() {
    let out = run(&[
        create("a", json!({})),
        create("b", json!({})),
        send("a", "one"),
        send("b", "two"),
        tick(7),
        json!({"Command": "ReceiveMessage", "QueueName": "b", "MaxMessages": 3}),
        receive("a"),
    ]);
    assert_eq!(out[2], json!({"ok": true, "MessageId": "m-1"}));
    assert_eq!(out[3], json!({"ok": true, "MessageId": "m-2"}));
    assert_eq!(
        out[5],
        json!({"ok": true, "Messages": [{
            "MessageId": "m-2", "ReceiptHandle": "r-1", "Body": "two",
            "ReceiveCount": 1, "SentAt": 0, "FirstReceivedAt": 7
        }]})
    );
    assert_eq!(out[6]["Messages"][0]["ReceiptHandle"], "r-2");
}

#[test]
fn send_validation() {
    let out = run(&[
        create("q", json!({})),
        send("q", ""),
        send("q", &"x".repeat(1025)),
        send("q", &"é".repeat(512)),
        send("q", &"é".repeat(513)),
        json!({"Command": "SendMessage", "QueueName": "q", "Body": "b", "DelaySeconds": 901}),
        json!({"Command": "SendMessage", "QueueName": "q", "Body": "b", "GroupId": "g"}),
        json!({"Command": "SendMessage", "QueueName": "q", "Body": "b", "DeduplicationId": "d"}),
        json!({"Command": "SendMessage", "QueueName": "q", "Body": "b", "GroupId": "bad id"}),
        send("q", "ok"),
    ]);
    let codes: Vec<_> = out[1..]
        .iter()
        .map(|r| r["error"]["code"].as_str().unwrap_or("ok"))
        .collect();
    assert_eq!(
        codes,
        [
            "InvalidParameterValue",
            "InvalidParameterValue",
            "ok",
            "InvalidParameterValue",
            "InvalidParameterValue",
            "UnsupportedOperation",
            "UnsupportedOperation",
            "InvalidParameterValue",
            "ok",
        ]
    );
    // Failed sends do not use up message ids.
    assert_eq!(out[9]["MessageId"], "m-2");
}

#[test]
fn delay_hides_messages_until_it_has_passed() {
    let out = run(&[
        create("q", json!({"DelaySeconds": 10})),
        send("q", "queue-delay"),
        json!({"Command": "SendMessage", "QueueName": "q", "Body": "own-delay", "DelaySeconds": 0}),
        counts("q"),
        receive("q"),
        tick(10),
        counts("q"),
    ]);
    assert_eq!(
        out[3]["Counts"],
        json!({"Visible": 1, "InFlight": 0, "Delayed": 1})
    );
    assert_eq!(out[4]["Messages"][0]["Body"], "own-delay");
    assert_eq!(
        out[6]["Counts"],
        json!({"Visible": 1, "InFlight": 1, "Delayed": 0})
    );
}

#[test]
fn visibility_timeout_returns_message_to_its_place_in_arrival_order() {
    let out = run(&[
        create("q", json!({"VisibilityTimeout": 5})),
        send("q", "first"),
        send("q", "second"),
        receive("q"),
        receive("q"),
        tick(5),
        receive("q"),
    ]);
    assert_eq!(out[3]["Messages"][0]["Body"], "first");
    assert_eq!(out[4]["Messages"][0]["Body"], "second");
    assert_eq!(out[6]["Messages"][0]["Body"], "first");
    assert_eq!(out[6]["Messages"][0]["ReceiveCount"], 2);
    assert_eq!(out[6]["Messages"][0]["FirstReceivedAt"], 0);
    assert_eq!(out[6]["Messages"][0]["ReceiptHandle"], "r-3");
}

#[test]
fn delete_and_change_visibility_error_precedence() {
    let handle =
        |h: &str| json!({"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": h});
    let out = run(&[
        create("q", json!({})),
        create("other", json!({})),
        send("q", "m"),
        send("other", "n"),
        receive("q"),
        receive("other"),
        handle("r-01"),
        handle("R-1"),
        handle("r-1 "),
        handle("r-3"),
        handle("r-2"),
        handle("r-1"),
        handle("r-1"),
        json!({"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": "r-9"}),
        json!({"Command": "DeleteMessage", "QueueName": "nope", "ReceiptHandle": "junk"}),
    ]);
    let codes: Vec<_> = out[6..]
        .iter()
        .map(|r| r["error"]["code"].as_str().unwrap_or("ok"))
        .collect();
    assert_eq!(
        codes,
        [
            "ReceiptHandleInvalid",
            "ReceiptHandleInvalid",
            "ReceiptHandleInvalid",
            "ReceiptHandleInvalid",
            "ReceiptHandleExpired", // issued for a message of another queue
            "ok",
            "ReceiptHandleExpired", // the message is gone
            "ReceiptHandleInvalid",
            "QueueDoesNotExist",
        ]
    );
}

#[test]
fn change_visibility_moves_the_end_of_the_timeout() {
    let change = |h: &str, v: u32| json!({"Command": "ChangeMessageVisibility", "QueueName": "q", "ReceiptHandle": h, "VisibilityTimeout": v});
    let out = run(&[
        create("q", json!({"VisibilityTimeout": 10})),
        send("q", "m"),
        receive("q"),
        tick(4),
        change("r-1", 100),
        tick(50),
        counts("q"),
        change("r-1", 0),
        counts("q"),
        change("r-1", 5),
    ]);
    assert_eq!(out[4], json!({"ok": true}));
    assert_eq!(out[6]["Counts"]["InFlight"], 1);
    assert_eq!(out[7], json!({"ok": true}));
    assert_eq!(
        out[8]["Counts"],
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(error_code(&out[9]), "ReceiptHandleExpired");
}

#[test]
fn zero_visibility_timeout_makes_the_handle_unusable() {
    let out = run(&[
        create("q", json!({})),
        send("q", "m"),
        json!({"Command": "ReceiveMessage", "QueueName": "q", "VisibilityTimeout": 0}),
        json!({"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": "r-1"}),
        receive("q"),
    ]);
    assert_eq!(error_code(&out[3]), "ReceiptHandleExpired");
    assert_eq!(out[4]["Messages"][0]["ReceiveCount"], 2);
}

#[test]
fn retention_removes_messages_at_once_when_lowered() {
    let out = run(&[
        create("q", json!({"RetentionSeconds": 100})),
        send("q", "old"),
        tick(60),
        send("q", "young"),
        json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"RetentionSeconds": 60}}),
        counts("q"),
        json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"RetentionSeconds": 1000}}),
        counts("q"),
        tick(999),
        counts("q"),
        tick(1),
        counts("q"),
    ]);
    assert_eq!(out[5]["Counts"]["Visible"], 1);
    assert_eq!(
        out[7]["Counts"]["Visible"], 1,
        "raising retention does not bring messages back"
    );
    assert_eq!(out[9]["Counts"]["Visible"], 1);
    assert_eq!(out[11]["Counts"]["Visible"], 0);
}

#[test]
fn purge_removes_everything_but_keeps_counters() {
    let out = run(&[
        create("f.fifo", json!({"Fifo": true})),
        json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "a", "GroupId": "g", "DeduplicationId": "d"}),
        json!({"Command": "PurgeQueue", "QueueName": "f.fifo"}),
        json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "a", "GroupId": "g", "DeduplicationId": "d"}),
        json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "b", "GroupId": "g", "DeduplicationId": "e"}),
    ]);
    assert_eq!(
        out[3],
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
    assert_eq!(
        out[4],
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2})
    );
}
