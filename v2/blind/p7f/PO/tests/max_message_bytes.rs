mod common;

use common::Harness;
use serde_json::json;

fn create(h: &mut Harness, name: &str, max: u64) {
    h.ok(json!({"Command": "CreateQueue", "QueueName": name,
                "Attributes": {"MaxMessageBytes": max}}));
}

fn send(h: &mut Harness, queue: &str, body: &str) -> serde_json::Value {
    h.run(json!({"Command": "SendMessage", "QueueName": queue, "Body": body}))
}

#[test]
fn body_is_limited_in_utf8_bytes() {
    let mut h = Harness::new();
    create(&mut h, "q", 4);
    assert_eq!(
        send(&mut h, "q", "abcd"),
        json!({"ok": true, "MessageId": "m-1"})
    );
    // Two characters, five bytes.
    assert_eq!(
        send(&mut h, "q", "ab\u{e9}\u{e9}")["error"]["code"],
        "InvalidParameterValue"
    );
    assert_eq!(send(&mut h, "q", "\u{e9}\u{e9}")["ok"], true);
    // A failed send does not use up a message id.
    assert_eq!(send(&mut h, "q", "x")["MessageId"], "m-3");
}

#[test]
fn attribute_range_and_default() {
    let mut h = Harness::new();
    for bad in [json!(0), json!(1025), json!("5"), json!(5.5)] {
        assert_eq!(
            h.err(json!({"Command": "CreateQueue", "QueueName": "q",
                         "Attributes": {"MaxMessageBytes": bad}})),
            "InvalidParameterValue"
        );
    }
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    let get = |h: &mut Harness| {
        h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}))["Attributes"]
            ["MaxMessageBytes"]
            .clone()
    };
    assert_eq!(get(&mut h), 1024);
    h.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q",
                "Attributes": {"MaxMessageBytes": 1}}));
    assert_eq!(get(&mut h), 1);
}

#[test]
fn create_again_compares_max_message_bytes() {
    let mut h = Harness::new();
    create(&mut h, "q", 10);
    create(&mut h, "q", 10);
    assert_eq!(
        h.err(json!({"Command": "CreateQueue", "QueueName": "q"})),
        "QueueAlreadyExists"
    );
}

#[test]
fn lowering_the_limit_keeps_queued_messages() {
    let mut h = Harness::new();
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    send(&mut h, "q", "long body");
    h.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q",
                "Attributes": {"MaxMessageBytes": 2}}));
    let r = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q"}));
    assert_eq!(r["Messages"][0]["Body"], "long body");
}

#[test]
fn limit_is_checked_after_the_queue_lookup_and_before_rule_one() {
    let mut h = Harness::new();
    assert_eq!(
        send(&mut h, "none", "x")["error"]["code"],
        "QueueDoesNotExist"
    );
    create(&mut h, "q", 1);
    // Standard queue with a GroupId would be UnsupportedOperation, but size comes first.
    assert_eq!(
        h.err(json!({"Command": "SendMessage", "QueueName": "q", "Body": "xx", "GroupId": "g"})),
        "InvalidParameterValue"
    );
}

#[test]
fn dead_letter_moves_ignore_the_limit() {
    let mut h = Harness::new();
    create(&mut h, "dead", 1);
    h.ok(json!({"Command": "CreateQueue", "QueueName": "src",
                "Attributes": {"DeadLetterQueue": "dead", "MaxReceiveCount": 1,
                               "VisibilityTimeout": 0}}));
    send(&mut h, "src", "too long for dead");
    h.ok(json!({"Command": "ReceiveMessage", "QueueName": "src"}));
    h.ok(json!({"Command": "ReceiveMessage", "QueueName": "src"}));
    let r = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "dead"}));
    assert_eq!(r["Messages"][0]["Body"], "too long for dead");
}
