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
fn limit_counts_utf8_bytes() {
    let mut h = Harness::new();
    create(&mut h, "q", 4);
    assert_eq!(send(&mut h, "q", "abcd")["ok"], true);
    assert_eq!(send(&mut h, "q", "ab\u{e9}")["ok"], true);
    // Three characters, five bytes.
    assert_eq!(
        send(&mut h, "q", "ab\u{e9}\u{e9}")["error"]["code"],
        "InvalidParameterValue"
    );
    assert_eq!(
        send(&mut h, "q", "abcde")["error"]["code"],
        "InvalidParameterValue"
    );
}

#[test]
fn rejected_send_uses_no_message_id() {
    let mut h = Harness::new();
    create(&mut h, "q", 1);
    assert_eq!(send(&mut h, "q", "xx")["ok"], false);
    assert_eq!(send(&mut h, "q", "x")["MessageId"], "m-1");
}

#[test]
fn limit_comes_before_the_type_rules() {
    let mut h = Harness::new();
    create(&mut h, "q", 1);
    // GroupId would be UnsupportedOperation on a standard queue.
    assert_eq!(
        h.err(json!({"Command": "SendMessage", "QueueName": "q", "Body": "xx", "GroupId": "g"})),
        "InvalidParameterValue"
    );
}

#[test]
fn body_rule_of_1024_bytes_still_comes_first_in_step_three() {
    let mut h = Harness::new();
    create(&mut h, "q", 1);
    h.ok(json!({"Command": "CreateQueue", "QueueName": "other"}));
    let long = "x".repeat(1025);
    assert_eq!(
        send(&mut h, "q", &long)["error"]["code"],
        "InvalidParameterValue"
    );
    // A missing queue is reported before the per-queue limit.
    assert_eq!(
        h.err(json!({"Command": "SendMessage", "QueueName": "none", "Body": "xx"})),
        "QueueDoesNotExist"
    );
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
    let r = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(r["Attributes"]["MaxMessageBytes"], 1024);
}

#[test]
fn create_again_compares_the_attribute() {
    let mut h = Harness::new();
    create(&mut h, "q", 10);
    create(&mut h, "q", 10);
    assert_eq!(
        h.err(json!({"Command": "CreateQueue", "QueueName": "q"})),
        "QueueAlreadyExists"
    );
    assert_eq!(
        h.err(json!({"Command": "CreateQueue", "QueueName": "q",
                     "Attributes": {"MaxMessageBytes": 11}})),
        "QueueAlreadyExists"
    );
}

#[test]
fn changing_the_limit_keeps_old_messages_and_applies_to_new_sends() {
    let mut h = Harness::new();
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    assert_eq!(send(&mut h, "q", "long body")["MessageId"], "m-1");
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
fn dead_letter_moves_ignore_the_limit() {
    let mut h = Harness::new();
    create(&mut h, "dlq", 1);
    h.ok(json!({"Command": "CreateQueue", "QueueName": "src",
                "Attributes": {"DeadLetterQueue": "dlq", "MaxReceiveCount": 1,
                               "VisibilityTimeout": 0}}));
    send(&mut h, "src", "long body");
    h.ok(json!({"Command": "ReceiveMessage", "QueueName": "src"}));
    h.ok(json!({"Command": "ReceiveMessage", "QueueName": "src"}));
    let r = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "dlq"}));
    assert_eq!(r["Messages"][0]["Body"], "long body");
}
