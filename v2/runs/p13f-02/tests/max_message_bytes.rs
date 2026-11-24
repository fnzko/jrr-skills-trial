mod common;

use common::Harness;
use serde_json::json;

fn create_with(h: &mut Harness, name: &str, max: u64) {
    h.ok(json!({"Command": "CreateQueue", "QueueName": name, "Attributes": {"MaxMessageBytes": max}}));
}

fn send(h: &mut Harness, body: &str) -> serde_json::Value {
    h.run(json!({"Command": "SendMessage", "QueueName": "q", "Body": body}))
}

#[test]
fn body_longer_than_limit_is_rejected_in_utf8_bytes() {
    let mut h = Harness::new();
    create_with(&mut h, "q", 4);
    assert_eq!(send(&mut h, "abcd")["ok"], true);
    assert_eq!(
        send(&mut h, "abcde")["error"]["code"],
        "InvalidParameterValue"
    );
    // Two characters but five bytes.
    assert_eq!(
        send(&mut h, "éé a")["error"]["code"],
        "InvalidParameterValue"
    );
    assert_eq!(send(&mut h, "éé")["ok"], true);
}

#[test]
fn limit_rule_comes_before_fifo_rules() {
    let mut h = Harness::new();
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q.fifo",
        "Attributes": {"Fifo": true, "MaxMessageBytes": 1}}));
    // No GroupId would be MissingParameter, but the size rule is checked first.
    assert_eq!(
        h.err(json!({"Command": "SendMessage", "QueueName": "q.fifo", "Body": "ab"})),
        "InvalidParameterValue"
    );
}

#[test]
fn attribute_range_is_1_to_1024() {
    let mut h = Harness::new();
    for bad in [0, 1025] {
        assert_eq!(
            h.err(json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": {"MaxMessageBytes": bad}})),
            "InvalidParameterValue"
        );
    }
    create_with(&mut h, "q", 1);
    create_with(&mut h, "q", 1);
    assert_eq!(
        h.err(json!({"Command": "CreateQueue", "QueueName": "q"})),
        "QueueAlreadyExists"
    );
}

#[test]
fn set_attributes_applies_to_later_sends_only() {
    let mut h = Harness::new();
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    assert_eq!(send(&mut h, "long body")["ok"], true);
    h.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"MaxMessageBytes": 3}}));
    assert_eq!(
        send(&mut h, "long body")["error"]["code"],
        "InvalidParameterValue"
    );
    let r = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(r["Attributes"]["MaxMessageBytes"], 3);
    assert_eq!(r["Counts"]["Visible"], 1);
}

#[test]
fn dead_letter_move_ignores_the_limit() {
    let mut h = Harness::new();
    h.ok(
        json!({"Command": "CreateQueue", "QueueName": "dlq", "Attributes": {"MaxMessageBytes": 1}}),
    );
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q",
        "Attributes": {"DeadLetterQueue": "dlq", "MaxReceiveCount": 1}}));
    send(&mut h, "long body");
    h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q", "VisibilityTimeout": 0}));
    h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q"}));
    let r = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "dlq"}));
    assert_eq!(r["Messages"][0]["Body"], "long body");
}
