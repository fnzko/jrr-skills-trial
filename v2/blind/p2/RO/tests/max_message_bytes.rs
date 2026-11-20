mod common;

use common::Harness;
use serde_json::json;

fn create(h: &mut Harness, name: &str, max: u64) {
    h.ok(json!({"Command": "CreateQueue", "QueueName": name,
                "Attributes": {"MaxMessageBytes": max}}));
}

fn send(h: &mut Harness, queue: &str, body: &str) -> String {
    h.run(json!({"Command": "SendMessage", "QueueName": queue, "Body": body}))["error"]["code"]
        .as_str()
        .unwrap_or("ok")
        .to_owned()
}

#[test]
fn body_longer_than_limit_is_rejected_by_utf8_bytes() {
    let mut h = Harness::new();
    create(&mut h, "q", 4);
    assert_eq!(send(&mut h, "q", "abcd"), "ok");
    assert_eq!(send(&mut h, "q", "abcde"), "InvalidParameterValue");
    // "é" is 2 bytes: two fit, three do not.
    assert_eq!(send(&mut h, "q", "éé"), "ok");
    assert_eq!(send(&mut h, "q", "ééé"), "InvalidParameterValue");
}

#[test]
fn range_and_default() {
    let mut h = Harness::new();
    for bad in [json!(0), json!(1025), json!("5"), json!(5.0)] {
        assert_eq!(
            h.err(json!({"Command": "CreateQueue", "QueueName": "q",
                         "Attributes": {"MaxMessageBytes": bad}})),
            "InvalidParameterValue"
        );
    }
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    let r = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(r["Attributes"]["MaxMessageBytes"], 1024);
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
fn set_attributes_does_not_touch_existing_messages() {
    let mut h = Harness::new();
    create(&mut h, "q", 100);
    assert_eq!(send(&mut h, "q", "0123456789"), "ok");
    h.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q",
                "Attributes": {"MaxMessageBytes": 3}}));
    assert_eq!(send(&mut h, "q", "0123456789"), "InvalidParameterValue");
    let r = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q"}));
    assert_eq!(r["Messages"][0]["Body"], "0123456789");
}

#[test]
fn rule_comes_before_fifo_rules_but_after_lookup_and_body_range() {
    let mut h = Harness::new();
    h.ok(json!({"Command": "CreateQueue", "QueueName": "f.fifo",
                "Attributes": {"Fifo": true, "MaxMessageBytes": 2}}));
    // Would be MissingParameter (no GroupId) without the size rule.
    assert_eq!(
        h.err(json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "abc"})),
        "InvalidParameterValue"
    );
    assert_eq!(
        h.err(json!({"Command": "SendMessage", "QueueName": "nope", "Body": "abc"})),
        "QueueDoesNotExist"
    );
}

#[test]
fn dead_letter_moves_ignore_the_limit() {
    let mut h = Harness::new();
    create(&mut h, "dlq", 2);
    h.ok(json!({"Command": "CreateQueue", "QueueName": "src",
                "Attributes": {"DeadLetterQueue": "dlq", "MaxReceiveCount": 1,
                               "VisibilityTimeout": 0}}));
    assert_eq!(send(&mut h, "src", "long body"), "ok");
    h.ok(json!({"Command": "ReceiveMessage", "QueueName": "src"}));
    h.ok(json!({"Command": "ReceiveMessage", "QueueName": "src"}));
    let r = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "dlq"}));
    assert_eq!(r["Messages"][0]["Body"], "long body");
}
