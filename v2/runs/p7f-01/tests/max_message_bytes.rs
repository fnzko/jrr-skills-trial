mod common;

use common::Harness;
use serde_json::json;

fn create(h: &mut Harness, name: &str, attributes: serde_json::Value) {
    h.ok(json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attributes}));
}

fn send(body: &str) -> serde_json::Value {
    json!({"Command": "SendMessage", "QueueName": "q", "Body": body})
}

fn max_bytes(h: &mut Harness) -> serde_json::Value {
    h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}))["Attributes"]
        ["MaxMessageBytes"]
        .clone()
}

#[test]
fn limit_counts_utf8_bytes_and_is_inclusive() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({"MaxMessageBytes": 4}));
    h.ok(send("abcd"));
    assert_eq!(h.err(send("abcde")), "InvalidParameterValue");
    // Two characters, four bytes.
    h.ok(send("éé"));
    // One character more than fits: three characters, six bytes.
    assert_eq!(h.err(send("ééé")), "InvalidParameterValue");
}

#[test]
fn failed_send_uses_no_message_id() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({"MaxMessageBytes": 1}));
    assert_eq!(h.err(send("ab")), "InvalidParameterValue");
    assert_eq!(h.ok(send("a"))["MessageId"], "m-1");
}

#[test]
fn attribute_range_is_one_to_1024() {
    let mut h = Harness::new();
    for bad in [json!(0), json!(1025), json!("5"), json!(5.0)] {
        assert_eq!(
            h.err(json!({"Command": "CreateQueue", "QueueName": "q",
                         "Attributes": {"MaxMessageBytes": bad}})),
            "InvalidParameterValue"
        );
    }
    create(&mut h, "q", json!({"MaxMessageBytes": 1}));
    assert_eq!(max_bytes(&mut h), json!(1));
}

#[test]
fn defaults_to_1024_and_takes_part_in_create_idempotence() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    assert_eq!(max_bytes(&mut h), json!(1024));
    create(&mut h, "q", json!({"MaxMessageBytes": 1024}));
    assert_eq!(
        h.err(json!({"Command": "CreateQueue", "QueueName": "q",
                     "Attributes": {"MaxMessageBytes": 10}})),
        "QueueAlreadyExists"
    );
}

#[test]
fn set_attributes_changes_limit_for_later_sends_only() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    h.ok(send("long body"));
    h.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q",
                "Attributes": {"MaxMessageBytes": 3}}));
    assert_eq!(max_bytes(&mut h), json!(3));
    assert_eq!(h.err(send("long body")), "InvalidParameterValue");
    let received = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q"}));
    assert_eq!(received["Messages"][0]["Body"], "long body");
}

#[test]
fn limit_is_checked_before_unsupported_operation() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({"MaxMessageBytes": 1}));
    assert_eq!(
        h.err(
            json!({"Command": "SendMessage", "QueueName": "q", "Body": "ab",
                     "GroupId": "g"})
        ),
        "InvalidParameterValue"
    );
}

#[test]
fn dead_letter_moves_ignore_the_limit() {
    let mut h = Harness::new();
    create(&mut h, "dlq", json!({"MaxMessageBytes": 1}));
    create(
        &mut h,
        "q",
        json!({"VisibilityTimeout": 0, "DeadLetterQueue": "dlq", "MaxReceiveCount": 1}),
    );
    h.ok(send("long body"));
    h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q"}));
    h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q"}));
    let moved = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "dlq"}));
    assert_eq!(moved["Messages"][0]["Body"], "long body");
}
