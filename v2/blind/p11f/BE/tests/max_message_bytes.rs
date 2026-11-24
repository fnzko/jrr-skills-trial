mod common;

use common::Harness;
use serde_json::json;

fn create_with(h: &mut Harness, attributes: serde_json::Value) {
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": attributes}));
}

fn send(h: &mut Harness, body: &str) -> serde_json::Value {
    h.run(json!({"Command": "SendMessage", "QueueName": "q", "Body": body}))
}

#[test]
fn default_is_1024_and_shown() {
    let mut h = Harness::new();
    create_with(&mut h, json!({}));
    let r = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(r["Attributes"]["MaxMessageBytes"], 1024);
}

#[test]
fn body_is_limited_in_utf8_bytes() {
    let mut h = Harness::new();
    create_with(&mut h, json!({"MaxMessageBytes": 4}));
    assert_eq!(send(&mut h, "abcd")["ok"], true);
    // Three characters, five bytes.
    assert_eq!(
        send(&mut h, "aéé")["error"]["code"],
        "InvalidParameterValue"
    );
    assert_eq!(send(&mut h, "éé")["ok"], true);
}

#[test]
fn limit_comes_before_other_send_rules() {
    let mut h = Harness::new();
    create_with(&mut h, json!({"MaxMessageBytes": 1}));
    let r =
        h.run(json!({"Command": "SendMessage", "QueueName": "q", "Body": "ab", "GroupId": "g"}));
    assert_eq!(r["error"]["code"], "InvalidParameterValue");
}

#[test]
fn range_is_checked() {
    let mut h = Harness::new();
    for bad in [json!(0), json!(1025), json!("5"), json!(5.0)] {
        let code = h.err(json!({"Command": "CreateQueue", "QueueName": "q",
            "Attributes": {"MaxMessageBytes": bad}}));
        assert_eq!(code, "InvalidParameterValue", "{bad}");
    }
    create_with(&mut h, json!({"MaxMessageBytes": 1024}));
}

#[test]
fn set_attributes_keeps_existing_messages() {
    let mut h = Harness::new();
    create_with(&mut h, json!({}));
    assert_eq!(send(&mut h, "long body")["ok"], true);
    h.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q",
        "Attributes": {"MaxMessageBytes": 3}}));
    let r = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q"}));
    assert_eq!(r["Messages"][0]["Body"], "long body");
    assert_eq!(
        send(&mut h, "long body")["error"]["code"],
        "InvalidParameterValue"
    );
}

#[test]
fn takes_part_in_create_idempotence() {
    let mut h = Harness::new();
    create_with(&mut h, json!({"MaxMessageBytes": 10}));
    create_with(&mut h, json!({"MaxMessageBytes": 10}));
    let code = h.err(json!({"Command": "CreateQueue", "QueueName": "q"}));
    assert_eq!(code, "QueueAlreadyExists");
}

#[test]
fn dead_letter_moves_ignore_the_limit() {
    let mut h = Harness::new();
    h.ok(json!({"Command": "CreateQueue", "QueueName": "dlq",
        "Attributes": {"MaxMessageBytes": 1}}));
    create_with(
        &mut h,
        json!({"DeadLetterQueue": "dlq", "MaxReceiveCount": 1,
        "VisibilityTimeout": 0}),
    );
    assert_eq!(send(&mut h, "longer").get("ok"), Some(&json!(true)));
    h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q"}));
    h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q"}));
    let r = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "dlq"}));
    assert_eq!(r["Messages"][0]["Body"], "longer");
}
