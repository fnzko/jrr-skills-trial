mod common;

use common::Harness;
use serde_json::json;

fn create(h: &mut Harness, name: &str, max: u64) {
    h.ok(json!({"Command": "CreateQueue", "QueueName": name, "Attributes": {"MaxMessageBytes": max}}));
}

fn send(h: &mut Harness, q: &str, body: &str) -> serde_json::Value {
    h.run(json!({"Command": "SendMessage", "QueueName": q, "Body": body}))
}

#[test]
fn attribute_range_and_default() {
    let mut h = Harness::new();
    for bad in [json!(0), json!(1025), json!("5"), json!(5.0)] {
        assert_eq!(
            h.err(json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": {"MaxMessageBytes": bad}})),
            "InvalidParameterValue"
        );
    }
    create(&mut h, "a", 1);
    create(&mut h, "b", 1024);
    h.ok(json!({"Command": "CreateQueue", "QueueName": "c"}));
    for (q, n) in [("a", 1), ("b", 1024), ("c", 1024)] {
        let r = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": q}));
        assert_eq!(r["Attributes"]["MaxMessageBytes"], n);
    }
}

#[test]
fn counts_utf8_bytes() {
    let mut h = Harness::new();
    create(&mut h, "q", 4);
    assert_eq!(send(&mut h, "q", "abcd")["ok"], true);
    assert_eq!(send(&mut h, "q", "ééé")["error"]["code"], "InvalidParameterValue"); // 6 bytes
    assert_eq!(send(&mut h, "q", "éé")["ok"], true); // 4 bytes
    assert_eq!(send(&mut h, "q", "abcde")["error"]["code"], "InvalidParameterValue");
}

#[test]
fn takes_part_in_create_idempotence() {
    let mut h = Harness::new();
    create(&mut h, "q", 10);
    create(&mut h, "q", 10);
    assert_eq!(
        h.err(json!({"Command": "CreateQueue", "QueueName": "q"})),
        "QueueAlreadyExists"
    );
}

#[test]
fn rule_comes_before_other_send_rules() {
    let mut h = Harness::new();
    create(&mut h, "q", 2);
    // would be UnsupportedOperation without the size rule
    let r = h.run(json!({"Command": "SendMessage", "QueueName": "q", "Body": "abc", "GroupId": "g"}));
    assert_eq!(r["error"]["code"], "InvalidParameterValue");
    // the 1024 rule of step 3 still wins over a missing queue
    let r = h.run(json!({"Command": "SendMessage", "QueueName": "nope", "Body": "x".repeat(1025)}));
    assert_eq!(r["error"]["code"], "InvalidParameterValue");
    let r = h.run(json!({"Command": "SendMessage", "QueueName": "nope", "Body": "abc"}));
    assert_eq!(r["error"]["code"], "QueueDoesNotExist");
}

#[test]
fn failed_send_uses_no_message_id() {
    let mut h = Harness::new();
    create(&mut h, "q", 1);
    assert_eq!(send(&mut h, "q", "toolong")["ok"], false);
    assert_eq!(send(&mut h, "q", "x")["MessageId"], "m-1");
}

#[test]
fn set_attributes_does_not_touch_existing_messages() {
    let mut h = Harness::new();
    create(&mut h, "q", 10);
    send(&mut h, "q", "0123456789");
    h.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"MaxMessageBytes": 3}}));
    let r = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q"}));
    assert_eq!(r["Messages"][0]["Body"], "0123456789");
    assert_eq!(send(&mut h, "q", "abcd")["ok"], false);
    h.err(json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"MaxMessageBytes": 0}}));
}

#[test]
fn dead_letter_moves_ignore_the_limit() {
    let mut h = Harness::new();
    create(&mut h, "dlq", 3);
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q",
        "Attributes": {"DeadLetterQueue": "dlq", "MaxReceiveCount": 1}}));
    send(&mut h, "q", "longer than three");
    h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q", "VisibilityTimeout": 0}));
    h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q"}));
    let r = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "dlq"}));
    assert_eq!(r["Messages"][0]["Body"], "longer than three");
}
