mod common;

use common::Harness;
use serde_json::json;

fn create(h: &mut Harness, attrs: serde_json::Value) {
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": attrs}));
}

fn send(h: &mut Harness, body: &str) -> Value2 {
    h.run(json!({"Command": "SendMessage", "QueueName": "q", "Body": body}))
}
type Value2 = serde_json::Value;

#[test]
fn default_is_1024_and_shown() {
    let mut h = Harness::new();
    create(&mut h, json!({}));
    let r = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(r["Attributes"]["MaxMessageBytes"], 1024);
}

#[test]
fn range_is_checked() {
    let mut h = Harness::new();
    for bad in [json!(0), json!(1025), json!("5"), json!(1.5)] {
        let c = json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": {"MaxMessageBytes": bad}});
        assert_eq!(h.err(c), "InvalidParameterValue");
    }
    create(&mut h, json!({"MaxMessageBytes": 1}));
}

#[test]
fn send_is_limited_in_utf8_bytes() {
    let mut h = Harness::new();
    create(&mut h, json!({"MaxMessageBytes": 4}));
    assert_eq!(send(&mut h, "abcd")["ok"], true);
    assert_eq!(send(&mut h, "abcde")["error"]["code"], "InvalidParameterValue");
    assert_eq!(send(&mut h, "éé")["ok"], true); // 4 bytes
    assert_eq!(send(&mut h, "ééa")["error"]["code"], "InvalidParameterValue");
}

#[test]
fn limit_rule_comes_before_rule_1() {
    let mut h = Harness::new();
    create(&mut h, json!({"MaxMessageBytes": 2}));
    let r = h.err(json!({"Command": "SendMessage", "QueueName": "q", "Body": "abc", "GroupId": "g"}));
    assert_eq!(r, "InvalidParameterValue");
}

#[test]
fn set_attributes_and_existing_messages() {
    let mut h = Harness::new();
    create(&mut h, json!({}));
    send(&mut h, "long body");
    h.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"MaxMessageBytes": 3}}));
    let r = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q"}));
    assert_eq!(r["Messages"][0]["Body"], "long body");
    assert_eq!(send(&mut h, "long")["ok"], false);
}

#[test]
fn takes_part_in_create_idempotence() {
    let mut h = Harness::new();
    create(&mut h, json!({"MaxMessageBytes": 10}));
    create(&mut h, json!({"MaxMessageBytes": 10}));
    let c = json!({"Command": "CreateQueue", "QueueName": "q"});
    assert_eq!(h.err(c), "QueueAlreadyExists");
}

#[test]
fn dead_letter_moves_ignore_it() {
    let mut h = Harness::new();
    h.ok(json!({"Command": "CreateQueue", "QueueName": "dlq", "Attributes": {"MaxMessageBytes": 1}}));
    create(&mut h, json!({"DeadLetterQueue": "dlq", "MaxReceiveCount": 1, "VisibilityTimeout": 0}));
    send(&mut h, "long body");
    h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q"}));
    h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q"}));
    let r = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "dlq"}));
    assert_eq!(r["Messages"][0]["Body"], "long body");
}
