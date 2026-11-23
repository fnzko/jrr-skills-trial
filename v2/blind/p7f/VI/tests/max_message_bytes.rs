mod common;

use common::Harness;
use serde_json::{Value, json};

fn create(h: &mut Harness, name: &str, attributes: Value) {
    h.ok(json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attributes}));
}

fn send(h: &mut Harness, queue: &str, body: &str) -> Value {
    h.run(json!({"Command": "SendMessage", "QueueName": queue, "Body": body}))
}

#[test]
fn default_is_1024_and_always_reported() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let r = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(r["Attributes"]["MaxMessageBytes"], 1024);
}

#[test]
fn range_is_validated() {
    let mut h = Harness::new();
    for bad in [json!(0), json!(1025), json!("5"), json!(5.0)] {
        let code = h.err(json!({"Command": "CreateQueue", "QueueName": "q",
            "Attributes": {"MaxMessageBytes": bad}}));
        assert_eq!(code, "InvalidParameterValue", "{bad}");
    }
    create(&mut h, "q", json!({"MaxMessageBytes": 1}));
    create(&mut h, "r", json!({"MaxMessageBytes": 1024}));
    let r = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(r["Attributes"]["MaxMessageBytes"], 1);
    let r = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "r"}));
    assert_eq!(r["Attributes"]["MaxMessageBytes"], 1024);
}

#[test]
fn send_over_limit_counts_utf8_bytes() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({"MaxMessageBytes": 4}));
    assert_eq!(send(&mut h, "q", "abcd")["ok"], true);
    let r = send(&mut h, "q", "\u{e9}\u{20ac}");
    assert_eq!(r["error"]["code"], "InvalidParameterValue");
    assert_eq!(send(&mut h, "q", "\u{e9}\u{e9}")["ok"], true);
    let r = send(&mut h, "q", "abcde");
    assert_eq!(r["error"]["code"], "InvalidParameterValue");
}

#[test]
fn limit_check_comes_before_send_rule_1() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({"MaxMessageBytes": 2}));
    let code = h.err(json!({"Command": "SendMessage", "QueueName": "q",
        "Body": "abc", "GroupId": "g"}));
    assert_eq!(code, "InvalidParameterValue");
    let code = h.err(json!({"Command": "SendMessage", "QueueName": "nope",
        "Body": "x".repeat(1025)}));
    assert_eq!(code, "InvalidParameterValue");
}

#[test]
fn set_attributes_does_not_affect_existing_messages() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    assert_eq!(send(&mut h, "q", "long body")["ok"], true);
    h.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q",
        "Attributes": {"MaxMessageBytes": 3}}));
    let r = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q"}));
    assert_eq!(r["Messages"][0]["Body"], "long body");
    assert_eq!(send(&mut h, "q", "long body")["ok"], false);
}

#[test]
fn takes_part_in_create_idempotence() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({"MaxMessageBytes": 10}));
    create(&mut h, "q", json!({"MaxMessageBytes": 10}));
    let code = h.err(json!({"Command": "CreateQueue", "QueueName": "q"}));
    assert_eq!(code, "QueueAlreadyExists");
}

#[test]
fn dead_letter_moves_ignore_the_limit() {
    let mut h = Harness::new();
    create(&mut h, "dlq", json!({"MaxMessageBytes": 1}));
    create(
        &mut h,
        "q",
        json!({"DeadLetterQueue": "dlq", "MaxReceiveCount": 1,
        "VisibilityTimeout": 0}),
    );
    assert_eq!(send(&mut h, "q", "much longer")["ok"], true);
    h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q"}));
    h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q"}));
    let r = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "dlq"}));
    assert_eq!(r["Messages"][0]["Body"], "much longer");
}
