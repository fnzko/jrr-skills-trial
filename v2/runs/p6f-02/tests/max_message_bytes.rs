mod common;

use common::Harness;
use serde_json::{Value, json};

fn create(h: &mut Harness, attributes: Value) {
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": attributes}));
}

fn send(h: &mut Harness, body: &str) -> String {
    h.err(json!({"Command": "SendMessage", "QueueName": "q", "Body": body}))
}

#[test]
fn body_longer_than_limit_is_rejected_by_utf8_bytes() {
    let mut h = Harness::new();
    create(&mut h, json!({"MaxMessageBytes": 4}));
    h.ok(json!({"Command": "SendMessage", "QueueName": "q", "Body": "abcd"}));
    assert_eq!(send(&mut h, "abcde"), "InvalidParameterValue");
    // Two characters, but four bytes: fits. Three characters, six bytes: does not.
    h.ok(json!({"Command": "SendMessage", "QueueName": "q", "Body": "\u{e9}\u{e9}"}));
    assert_eq!(send(&mut h, "\u{e9}\u{e9}\u{e9}"), "InvalidParameterValue");
}

#[test]
fn limit_check_comes_before_send_rules() {
    let mut h = Harness::new();
    create(&mut h, json!({"MaxMessageBytes": 1}));
    // A standard queue given a GroupId is UnsupportedOperation, but the size rule is first.
    let code =
        h.err(json!({"Command": "SendMessage", "QueueName": "q", "Body": "ab", "GroupId": "g"}));
    assert_eq!(code, "InvalidParameterValue");
}

#[test]
fn attribute_range_and_default() {
    let mut h = Harness::new();
    for bad in [json!(0), json!(1025), json!("5")] {
        let code = h.err(json!({"Command": "CreateQueue", "QueueName": "q",
                                "Attributes": {"MaxMessageBytes": bad}}));
        assert_eq!(code, "InvalidParameterValue");
    }
    create(&mut h, json!({"MaxMessageBytes": 1024}));
    let r = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(r["Attributes"]["MaxMessageBytes"], 1024);
}

#[test]
fn create_compares_max_message_bytes() {
    let mut h = Harness::new();
    create(&mut h, json!({"MaxMessageBytes": 10}));
    create(&mut h, json!({"MaxMessageBytes": 10}));
    let code = h.err(json!({"Command": "CreateQueue", "QueueName": "q"}));
    assert_eq!(code, "QueueAlreadyExists");
}

#[test]
fn lowering_limit_keeps_existing_messages() {
    let mut h = Harness::new();
    create(&mut h, json!({}));
    h.ok(json!({"Command": "SendMessage", "QueueName": "q", "Body": "long body"}));
    h.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q",
                "Attributes": {"MaxMessageBytes": 2}}));
    let r = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q"}));
    assert_eq!(r["Messages"][0]["Body"], "long body");
    assert_eq!(send(&mut h, "long body"), "InvalidParameterValue");
}

#[test]
fn dead_letter_move_ignores_limit() {
    let mut h = Harness::new();
    h.ok(json!({"Command": "CreateQueue", "QueueName": "dlq",
                "Attributes": {"MaxMessageBytes": 1}}));
    create(
        &mut h,
        json!({"DeadLetterQueue": "dlq", "MaxReceiveCount": 1}),
    );
    h.ok(json!({"Command": "SendMessage", "QueueName": "q", "Body": "long body"}));
    h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q", "VisibilityTimeout": 0}));
    h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q"}));
    let r = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "dlq"}));
    assert_eq!(r["Messages"][0]["Body"], "long body");
}
