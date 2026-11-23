mod common;

use common::Harness;
use serde_json::json;

fn create(h: &mut Harness, name: &str, max: u64) {
    h.ok(json!({"Command": "CreateQueue", "QueueName": name, "Attributes": {"MaxMessageBytes": max}}));
}

fn send(h: &mut Harness, body: &str) -> String {
    h.err(json!({"Command": "SendMessage", "QueueName": "q", "Body": body}))
}

#[test]
fn defaults_to_1024_and_is_reported() {
    let mut h = Harness::new();
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    let r = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(r["Attributes"]["MaxMessageBytes"], 1024);
}

#[test]
fn range_is_1_to_1024() {
    let mut h = Harness::new();
    for bad in [json!(0), json!(1025), json!("5"), json!(5.0)] {
        let code = h.err(json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": {"MaxMessageBytes": bad}}));
        assert_eq!(code, "InvalidParameterValue");
    }
    create(&mut h, "q", 1);
    h.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"MaxMessageBytes": 1024}}));
    assert_eq!(
        h.err(json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"MaxMessageBytes": 0}})),
        "InvalidParameterValue"
    );
}

#[test]
fn body_is_limited_in_utf8_bytes() {
    let mut h = Harness::new();
    create(&mut h, "q", 4);
    h.ok(json!({"Command": "SendMessage", "QueueName": "q", "Body": "abcd"}));
    h.ok(json!({"Command": "SendMessage", "QueueName": "q", "Body": "\u{e9}\u{e9}"}));
    assert_eq!(send(&mut h, "abcde"), "InvalidParameterValue");
    assert_eq!(send(&mut h, "\u{e9}\u{e9}a"), "InvalidParameterValue");
}

#[test]
fn limit_comes_before_unsupported_operation_and_failed_send_uses_no_id() {
    let mut h = Harness::new();
    create(&mut h, "q", 2);
    let code = h.err(json!({"Command": "SendMessage", "QueueName": "q", "Body": "abc", "GroupId": "g"}));
    assert_eq!(code, "InvalidParameterValue");
    let r = h.ok(json!({"Command": "SendMessage", "QueueName": "q", "Body": "ab"}));
    assert_eq!(r["MessageId"], "m-1");
}

#[test]
fn changing_the_limit_keeps_existing_messages() {
    let mut h = Harness::new();
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    h.ok(json!({"Command": "SendMessage", "QueueName": "q", "Body": "long body"}));
    h.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"MaxMessageBytes": 1}}));
    let r = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q"}));
    assert_eq!(r["Messages"][0]["Body"], "long body");
}

#[test]
fn dead_letter_moves_ignore_the_limit() {
    let mut h = Harness::new();
    create(&mut h, "dlq", 1);
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q",
        "Attributes": {"DeadLetterQueue": "dlq", "MaxReceiveCount": 1, "VisibilityTimeout": 0}}));
    h.ok(json!({"Command": "SendMessage", "QueueName": "q", "Body": "long body"}));
    h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q"}));
    h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q"}));
    let r = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "dlq"}));
    assert_eq!(r["Messages"][0]["Body"], "long body");
}

#[test]
fn create_again_compares_the_limit() {
    let mut h = Harness::new();
    create(&mut h, "q", 10);
    create(&mut h, "q", 10);
    let code = h.err(json!({"Command": "CreateQueue", "QueueName": "q"}));
    assert_eq!(code, "QueueAlreadyExists");
}
