mod common;

use common::Harness;
use serde_json::json;

fn create(h: &mut Harness, name: &str, attributes: serde_json::Value) {
    h.ok(json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attributes}));
}

fn send(h: &mut Harness, queue: &str, body: &str) -> serde_json::Value {
    h.run(json!({"Command": "SendMessage", "QueueName": queue, "Body": body}))
}

fn max_bytes(h: &mut Harness, queue: &str) -> serde_json::Value {
    h.ok(json!({"Command": "GetQueueAttributes", "QueueName": queue}))["Attributes"]
        ["MaxMessageBytes"]
        .clone()
}

#[test]
fn attribute_range_is_1_to_1024() {
    let mut h = Harness::new();
    for bad in [json!(0), json!(1025), json!(-1), json!("5"), json!(5.0)] {
        let command = json!({"Command": "CreateQueue", "QueueName": "q",
                             "Attributes": {"MaxMessageBytes": bad}});
        assert_eq!(h.err(command), "InvalidParameterValue", "{bad}");
    }
    create(&mut h, "lo", json!({"MaxMessageBytes": 1}));
    create(&mut h, "hi", json!({"MaxMessageBytes": 1024}));
    assert_eq!(max_bytes(&mut h, "lo"), 1);
    assert_eq!(max_bytes(&mut h, "hi"), 1024);
}

#[test]
fn body_longer_than_the_limit_is_rejected_in_bytes() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({"MaxMessageBytes": 4}));
    assert_eq!(send(&mut h, "q", "abcd")["MessageId"], "m-1");
    assert_eq!(
        send(&mut h, "q", "abcde")["error"]["code"],
        "InvalidParameterValue"
    );
    // Two characters, four bytes: fits. Three characters, six bytes: does not.
    assert_eq!(send(&mut h, "q", "éé")["MessageId"], "m-2");
    assert_eq!(
        send(&mut h, "q", "ééé")["error"]["code"],
        "InvalidParameterValue"
    );
}

#[test]
fn limit_check_comes_before_fifo_rules_and_after_lookup() {
    let mut h = Harness::new();
    create(
        &mut h,
        "q.fifo",
        json!({"Fifo": true, "MaxMessageBytes": 1}),
    );
    // No GroupId and a delay would be other errors, but the size rule is first.
    let long = json!({"Command": "SendMessage", "QueueName": "q.fifo", "Body": "ab",
                      "DelaySeconds": 1});
    assert_eq!(h.err(long), "InvalidParameterValue");
    let missing_queue = json!({"Command": "SendMessage", "QueueName": "nope", "Body": "ab"});
    assert_eq!(h.err(missing_queue), "QueueDoesNotExist");
}

#[test]
fn global_body_limit_still_applies_with_default_attribute() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    assert_eq!(send(&mut h, "q", &"x".repeat(1024))["ok"], true);
    assert_eq!(
        send(&mut h, "q", &"x".repeat(1025))["error"]["code"],
        "InvalidParameterValue"
    );
    assert_eq!(
        send(&mut h, "q", "")["error"]["code"],
        "InvalidParameterValue"
    );
}

#[test]
fn changing_the_limit_keeps_old_messages_and_affects_later_sends() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    send(&mut h, "q", "long body");
    h.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q",
                "Attributes": {"MaxMessageBytes": 3}}));
    assert_eq!(
        send(&mut h, "q", "long body")["error"]["code"],
        "InvalidParameterValue"
    );
    let r = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q"}));
    assert_eq!(r["Messages"][0]["Body"], "long body");
}

#[test]
fn create_with_same_limit_is_idempotent_and_different_limit_conflicts() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({"MaxMessageBytes": 10}));
    create(&mut h, "q", json!({"MaxMessageBytes": 10}));
    let different = json!({"Command": "CreateQueue", "QueueName": "q"});
    assert_eq!(h.err(different), "QueueAlreadyExists");
}

#[test]
fn dead_letter_moves_ignore_the_limit() {
    let mut h = Harness::new();
    create(&mut h, "dlq", json!({"MaxMessageBytes": 1}));
    create(
        &mut h,
        "src",
        json!({"DeadLetterQueue": "dlq", "MaxReceiveCount": 1,
                                  "VisibilityTimeout": 0}),
    );
    send(&mut h, "src", "longer than one");
    h.ok(json!({"Command": "ReceiveMessage", "QueueName": "src"}));
    h.ok(json!({"Command": "ReceiveMessage", "QueueName": "src"}));
    let r = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "dlq"}));
    assert_eq!(r["Messages"][0]["Body"], "longer than one");
}
