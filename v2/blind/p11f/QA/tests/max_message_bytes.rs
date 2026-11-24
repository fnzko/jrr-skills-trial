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
fn attribute_is_set_by_create_and_set_and_reported() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({"MaxMessageBytes": 10}));
    let r = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(r["Attributes"]["MaxMessageBytes"], 10);
    h.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q",
                "Attributes": {"MaxMessageBytes": 1}}));
    let r = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(r["Attributes"]["MaxMessageBytes"], 1);
}

#[test]
fn values_outside_1_to_1024_are_rejected() {
    let mut h = Harness::new();
    for bad in [json!(0), json!(1025), json!("5"), json!(5.5)] {
        let code = h.err(json!({"Command": "CreateQueue", "QueueName": "q",
                                "Attributes": {"MaxMessageBytes": bad}}));
        assert_eq!(code, "InvalidParameterValue", "{bad}");
    }
}

#[test]
fn body_longer_than_limit_in_bytes_is_rejected() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({"MaxMessageBytes": 4}));
    assert_eq!(send(&mut h, "q", "abcd")["ok"], true);
    let r = send(&mut h, "q", "abcde");
    assert_eq!(r["error"]["code"], "InvalidParameterValue");
    // Two bytes per character: 3 characters are 6 bytes.
    let r = send(&mut h, "q", "ééé");
    assert_eq!(r["error"]["code"], "InvalidParameterValue");
    assert_eq!(send(&mut h, "q", "éé")["ok"], true);
}

#[test]
fn limit_is_checked_before_fifo_rules() {
    let mut h = Harness::new();
    create(
        &mut h,
        "q.fifo",
        json!({"Fifo": true, "MaxMessageBytes": 1}),
    );
    // No GroupId: MissingParameter would follow, but the size rule comes first.
    let r = send(&mut h, "q.fifo", "ab");
    assert_eq!(r["error"]["code"], "InvalidParameterValue");
}

#[test]
fn lowering_the_limit_keeps_existing_messages() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    assert_eq!(send(&mut h, "q", "long body")["ok"], true);
    h.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q",
                "Attributes": {"MaxMessageBytes": 2}}));
    let r = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q"}));
    assert_eq!(r["Messages"][0]["Body"], "long body");
}

#[test]
fn dead_letter_moves_ignore_the_limit() {
    let mut h = Harness::new();
    create(&mut h, "dlq", json!({"MaxMessageBytes": 1}));
    create(
        &mut h,
        "q",
        json!({"DeadLetterQueue": "dlq", "MaxReceiveCount": 1, "VisibilityTimeout": 0}),
    );
    assert_eq!(send(&mut h, "q", "long body")["ok"], true);
    h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q"}));
    h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q"}));
    let r = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "dlq"}));
    assert_eq!(r["Messages"][0]["Body"], "long body");
}

#[test]
fn create_again_with_other_limit_is_a_conflict() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({"MaxMessageBytes": 5}));
    create(&mut h, "q", json!({"MaxMessageBytes": 5}));
    let code = h.err(json!({"Command": "CreateQueue", "QueueName": "q"}));
    assert_eq!(code, "QueueAlreadyExists");
}
