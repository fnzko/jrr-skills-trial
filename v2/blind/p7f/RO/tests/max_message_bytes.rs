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
fn default_is_1024_and_always_reported() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let r = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(r["Attributes"]["MaxMessageBytes"], 1024);
}

#[test]
fn range_is_1_to_1024() {
    let mut h = Harness::new();
    for bad in [json!(0), json!(1025), json!("5"), json!(5.5)] {
        let code = h.err(json!({"Command": "CreateQueue", "QueueName": "q",
            "Attributes": {"MaxMessageBytes": bad}}));
        assert_eq!(code, "InvalidParameterValue", "{bad}");
    }
    create(&mut h, "lo", json!({"MaxMessageBytes": 1}));
    create(&mut h, "hi", json!({"MaxMessageBytes": 1024}));
}

#[test]
fn body_is_limited_in_utf8_bytes() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({"MaxMessageBytes": 4}));
    assert_eq!(send(&mut h, "q", "abcd")["ok"], true);
    assert_eq!(
        send(&mut h, "q", "abcde")["error"]["code"],
        "InvalidParameterValue"
    );
    // Two characters, four bytes.
    assert_eq!(send(&mut h, "q", "éé")["ok"], true);
    // Two characters, six bytes.
    assert_eq!(
        send(&mut h, "q", "€€")["error"]["code"],
        "InvalidParameterValue"
    );
}

#[test]
fn limit_rule_comes_before_the_unsupported_operation_rule() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({"MaxMessageBytes": 1}));
    let code = h.err(
        json!({"Command": "SendMessage", "QueueName": "q", "Body": "ab",
        "GroupId": "g"}),
    );
    assert_eq!(code, "InvalidParameterValue");
}

#[test]
fn failed_send_uses_no_message_id() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({"MaxMessageBytes": 1}));
    assert_eq!(send(&mut h, "q", "ab")["ok"], false);
    assert_eq!(send(&mut h, "q", "a")["MessageId"], "m-1");
}

#[test]
fn lowering_the_limit_keeps_existing_messages() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    send(&mut h, "q", "long body");
    h.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q",
        "Attributes": {"MaxMessageBytes": 2}}));
    let r = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q"}));
    assert_eq!(r["Messages"][0]["Body"], "long body");
    assert_eq!(
        send(&mut h, "q", "long body")["error"]["code"],
        "InvalidParameterValue"
    );
}

#[test]
fn dead_letter_moves_ignore_the_limit() {
    let mut h = Harness::new();
    create(&mut h, "dead", json!({"MaxMessageBytes": 1}));
    create(
        &mut h,
        "q",
        json!({"DeadLetterQueue": "dead", "MaxReceiveCount": 1,
        "VisibilityTimeout": 0}),
    );
    send(&mut h, "q", "long body");
    h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q"}));
    h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q"}));
    let r = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "dead"}));
    assert_eq!(r["Messages"][0]["Body"], "long body");
}

#[test]
fn create_queue_compares_the_attribute() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({"MaxMessageBytes": 10}));
    create(&mut h, "q", json!({"MaxMessageBytes": 10}));
    let code = h.err(json!({"Command": "CreateQueue", "QueueName": "q"}));
    assert_eq!(code, "QueueAlreadyExists");
}
