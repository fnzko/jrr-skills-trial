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
fn body_longer_than_limit_is_rejected_in_bytes() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({"MaxMessageBytes": 4}));
    assert_eq!(send(&mut h, "q", "abcd")["ok"], true);
    assert_eq!(
        h.err(json!({"Command": "SendMessage", "QueueName": "q", "Body": "abcde"})),
        "InvalidParameterValue"
    );
    // two characters, four bytes
    assert_eq!(send(&mut h, "q", "éé")["ok"], true);
    // two characters, six bytes
    assert_eq!(
        h.err(json!({"Command": "SendMessage", "QueueName": "q", "Body": "€€"})),
        "InvalidParameterValue"
    );
}

#[test]
fn limit_comes_before_the_unsupported_operation_rule() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({"MaxMessageBytes": 1}));
    assert_eq!(
        h.err(json!({"Command": "SendMessage", "QueueName": "q", "Body": "ab", "GroupId": "g"})),
        "InvalidParameterValue"
    );
}

#[test]
fn attribute_range_default_and_set() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let get = |h: &mut Harness| {
        h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}))["Attributes"]
            ["MaxMessageBytes"]
            .clone()
    };
    assert_eq!(get(&mut h), json!(1024));
    for bad in [json!(0), json!(1025), json!("5"), json!(1.5)] {
        assert_eq!(
            h.err(json!({"Command": "SetQueueAttributes", "QueueName": "q",
                         "Attributes": {"MaxMessageBytes": bad}})),
            "InvalidParameterValue"
        );
    }
    h.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q",
                "Attributes": {"MaxMessageBytes": 1}}));
    assert_eq!(get(&mut h), json!(1));
    // the 1024-byte rule on Body stays
    h.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q",
                "Attributes": {"MaxMessageBytes": 1024}}));
    assert_eq!(send(&mut h, "q", &"x".repeat(1024))["ok"], true);
    assert_eq!(
        h.err(json!({"Command": "SendMessage", "QueueName": "q", "Body": "x".repeat(1025)})),
        "InvalidParameterValue"
    );
}

#[test]
fn create_again_compares_the_attribute() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({"MaxMessageBytes": 10}));
    create(&mut h, "q", json!({"MaxMessageBytes": 10}));
    assert_eq!(
        h.err(json!({"Command": "CreateQueue", "QueueName": "q"})),
        "QueueAlreadyExists"
    );
}

#[test]
fn lowering_the_limit_keeps_existing_messages() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    send(&mut h, "q", "long body");
    h.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q",
                "Attributes": {"MaxMessageBytes": 1}}));
    let r = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q"}));
    assert_eq!(r["Messages"][0]["Body"], "long body");
}

#[test]
fn dead_letter_moves_ignore_the_limit() {
    let mut h = Harness::new();
    create(&mut h, "dlq", json!({"MaxMessageBytes": 1}));
    create(
        &mut h,
        "src",
        json!({"DeadLetterQueue": "dlq", "MaxReceiveCount": 1, "VisibilityTimeout": 0}),
    );
    send(&mut h, "src", "long body");
    h.ok(json!({"Command": "ReceiveMessage", "QueueName": "src"}));
    h.ok(json!({"Command": "ReceiveMessage", "QueueName": "src"}));
    let r = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "dlq"}));
    assert_eq!(r["Messages"][0]["Body"], "long body");
}
