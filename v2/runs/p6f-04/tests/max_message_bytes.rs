mod common;

use common::Harness;
use serde_json::json;

fn create(h: &mut Harness, name: &str, attributes: serde_json::Value) {
    h.ok(json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attributes}));
}

fn send(h: &mut Harness, queue: &str, body: &str) -> serde_json::Value {
    h.run(json!({"Command": "SendMessage", "QueueName": queue, "Body": body}))
}

#[test]
fn default_is_1024_and_is_always_reported() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let r = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(r["Attributes"]["MaxMessageBytes"], 1024);
}

#[test]
fn attribute_range_is_1_to_1024() {
    let mut h = Harness::new();
    for bad in [json!(0), json!(1025), json!("5"), json!(5.0)] {
        let code = h.err(json!({"Command": "CreateQueue", "QueueName": "q",
            "Attributes": {"MaxMessageBytes": bad}}));
        assert_eq!(code, "InvalidParameterValue", "{bad}");
    }
    create(&mut h, "lo", json!({"MaxMessageBytes": 1}));
    create(&mut h, "hi", json!({"MaxMessageBytes": 1024}));
}

#[test]
fn send_rejects_bodies_over_the_limit_counted_in_utf8_bytes() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({"MaxMessageBytes": 4}));
    assert_eq!(send(&mut h, "q", "abcd")["ok"], true);
    let over = send(&mut h, "q", "abcde");
    assert_eq!(over["error"]["code"], "InvalidParameterValue");
    // Two characters, four bytes: allowed. One more byte is not.
    assert_eq!(send(&mut h, "q", "\u{e9}\u{e9}")["ok"], true);
    assert_eq!(
        send(&mut h, "q", "\u{e9}\u{e9}a")["error"]["code"],
        "InvalidParameterValue"
    );
    // The rejected sends used no message id.
    assert_eq!(send(&mut h, "q", "x")["MessageId"], "m-3");
}

#[test]
fn limit_rule_comes_before_the_other_send_rules() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({"MaxMessageBytes": 2}));
    let code = h.err(
        json!({"Command": "SendMessage", "QueueName": "q", "Body": "abc",
        "GroupId": "g"}),
    );
    assert_eq!(code, "InvalidParameterValue");
    create(
        &mut h,
        "f.fifo",
        json!({"Fifo": true, "MaxMessageBytes": 2}),
    );
    let code = h.err(json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "abc"}));
    assert_eq!(code, "InvalidParameterValue");
}

#[test]
fn set_attributes_changes_the_limit_but_keeps_existing_messages() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    send(&mut h, "q", "abcdef");
    h.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q",
        "Attributes": {"MaxMessageBytes": 3}}));
    let r = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q"}));
    assert_eq!(r["Messages"][0]["Body"], "abcdef");
    assert_eq!(
        send(&mut h, "q", "abcd")["error"]["code"],
        "InvalidParameterValue"
    );
}

#[test]
fn create_is_idempotent_only_for_an_equal_limit() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({"MaxMessageBytes": 10}));
    create(&mut h, "q", json!({"MaxMessageBytes": 10}));
    let code = h.err(json!({"Command": "CreateQueue", "QueueName": "q"}));
    assert_eq!(code, "QueueAlreadyExists");
}

#[test]
fn dead_letter_moves_ignore_the_limit() {
    let mut h = Harness::new();
    create(&mut h, "dlq", json!({"MaxMessageBytes": 2}));
    create(
        &mut h,
        "q",
        json!({"DeadLetterQueue": "dlq", "MaxReceiveCount": 1}),
    );
    send(&mut h, "q", "abcdef");
    h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q", "VisibilityTimeout": 0}));
    h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q"}));
    let r = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "dlq"}));
    assert_eq!(r["Messages"][0]["Body"], "abcdef");
}
