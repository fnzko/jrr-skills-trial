mod common;

use common::Harness;
use serde_json::json;

fn create_with(h: &mut Harness, name: &str, max: u64) {
    h.ok(json!({"Command": "CreateQueue", "QueueName": name, "Attributes": {"MaxMessageBytes": max}}));
}

fn send(h: &mut Harness, body: &str) -> String {
    h.err(json!({"Command": "SendMessage", "QueueName": "q", "Body": body}))
}

#[test]
fn attribute_range_and_visibility() {
    let mut h = Harness::new();
    for bad in [json!(0), json!(1025), json!("5"), json!(5.0)] {
        let r = h.err(json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": {"MaxMessageBytes": bad}}));
        assert_eq!(r, "InvalidParameterValue");
    }
    create_with(&mut h, "q", 1);
    let r = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(r["Attributes"]["MaxMessageBytes"], 1);
    // Part of CreateQueue rule 4.
    create_with(&mut h, "q", 1);
    assert_eq!(
        h.err(json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": {"MaxMessageBytes": 2}})),
        "QueueAlreadyExists"
    );
    assert_eq!(
        h.err(json!({"Command": "CreateQueue", "QueueName": "q"})),
        "QueueAlreadyExists"
    );
}

#[test]
fn send_is_limited_in_utf8_bytes() {
    let mut h = Harness::new();
    create_with(&mut h, "q", 3);
    h.ok(json!({"Command": "SendMessage", "QueueName": "q", "Body": "abc"}));
    assert_eq!(send(&mut h, "abcd"), "InvalidParameterValue");
    // "é" is two bytes
    h.ok(json!({"Command": "SendMessage", "QueueName": "q", "Body": "aé"}));
    assert_eq!(send(&mut h, "éé"), "InvalidParameterValue");
}

#[test]
fn rule_comes_before_other_send_rules() {
    let mut h = Harness::new();
    create_with(&mut h, "q", 1);
    // Would be UnsupportedOperation (standard queue with GroupId) otherwise.
    assert_eq!(
        h.err(json!({"Command": "SendMessage", "QueueName": "q", "Body": "ab", "GroupId": "g"})),
        "InvalidParameterValue"
    );
    // Step 3 still precedes it: an empty body is a plain value error, a missing queue wins.
    assert_eq!(
        h.err(json!({"Command": "SendMessage", "QueueName": "nope", "Body": "ab"})),
        "QueueDoesNotExist"
    );
}

#[test]
fn set_attributes_does_not_touch_existing_messages() {
    let mut h = Harness::new();
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    h.ok(json!({"Command": "SendMessage", "QueueName": "q", "Body": "long body"}));
    h.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"MaxMessageBytes": 2}}));
    let r = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q"}));
    assert_eq!(r["Messages"][0]["Body"], "long body");
    assert_eq!(send(&mut h, "abc"), "InvalidParameterValue");
}

#[test]
fn dead_letter_moves_ignore_the_limit() {
    let mut h = Harness::new();
    create_with(&mut h, "dlq", 1);
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q",
        "Attributes": {"DeadLetterQueue": "dlq", "MaxReceiveCount": 1}}));
    h.ok(json!({"Command": "SendMessage", "QueueName": "q", "Body": "toolong"}));
    h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q", "VisibilityTimeout": 0}));
    h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q"}));
    let r = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "dlq"}));
    assert_eq!(r["Messages"][0]["Body"], "toolong");
}
