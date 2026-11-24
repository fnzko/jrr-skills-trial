mod common;

use common::Harness;
use serde_json::json;

fn create_with_limit(h: &mut Harness, name: &str, limit: u64) {
    h.ok(json!({"Command": "CreateQueue", "QueueName": name,
                "Attributes": {"MaxMessageBytes": limit}}));
}

fn send(h: &mut Harness, queue: &str, body: &str) -> serde_json::Value {
    h.run(json!({"Command": "SendMessage", "QueueName": queue, "Body": body}))
}

#[test]
fn body_longer_than_limit_is_rejected_and_equal_is_accepted() {
    let mut h = Harness::new();
    create_with_limit(&mut h, "q", 4);
    assert_eq!(send(&mut h, "q", "abcd")["ok"], true);
    assert_eq!(
        h.err(json!({"Command": "SendMessage", "QueueName": "q", "Body": "abcde"})),
        "InvalidParameterValue"
    );
}

#[test]
fn limit_counts_utf8_bytes() {
    let mut h = Harness::new();
    create_with_limit(&mut h, "q", 3);
    assert_eq!(send(&mut h, "q", "é")["ok"], true);
    assert_eq!(
        h.err(json!({"Command": "SendMessage", "QueueName": "q", "Body": "éé"})),
        "InvalidParameterValue"
    );
}

#[test]
fn limit_rule_comes_before_the_unsupported_operation_rule() {
    let mut h = Harness::new();
    create_with_limit(&mut h, "q", 1);
    assert_eq!(
        h.err(
            json!({"Command": "SendMessage", "QueueName": "q", "Body": "ab",
                     "GroupId": "g"})
        ),
        "InvalidParameterValue"
    );
}

#[test]
fn body_rule_of_1024_bytes_still_applies_first_to_other_queues() {
    let mut h = Harness::new();
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    assert_eq!(send(&mut h, "q", &"a".repeat(1024))["ok"], true);
    assert_eq!(
        h.err(json!({"Command": "SendMessage", "QueueName": "q", "Body": "a".repeat(1025)})),
        "InvalidParameterValue"
    );
}

#[test]
fn attribute_range_is_1_to_1024() {
    let mut h = Harness::new();
    for bad in [0, 1025] {
        assert_eq!(
            h.err(json!({"Command": "CreateQueue", "QueueName": "q",
                         "Attributes": {"MaxMessageBytes": bad}})),
            "InvalidParameterValue"
        );
    }
    create_with_limit(&mut h, "q", 1);
    create_with_limit(&mut h, "q", 1);
    assert_eq!(
        h.err(json!({"Command": "CreateQueue", "QueueName": "q"})),
        "QueueAlreadyExists"
    );
}

#[test]
fn set_attributes_changes_limit_without_touching_stored_messages() {
    let mut h = Harness::new();
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    send(&mut h, "q", "long body");
    h.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q",
                "Attributes": {"MaxMessageBytes": 2}}));
    let attrs = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(attrs["Attributes"]["MaxMessageBytes"], 2);
    assert_eq!(attrs["Counts"]["Visible"], 1);
    let received = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q"}));
    assert_eq!(received["Messages"][0]["Body"], "long body");
}

#[test]
fn dead_letter_move_ignores_the_target_limit() {
    let mut h = Harness::new();
    create_with_limit(&mut h, "dead", 1);
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q",
                "Attributes": {"DeadLetterQueue": "dead", "MaxReceiveCount": 1,
                               "VisibilityTimeout": 0}}));
    send(&mut h, "q", "long body");
    h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q"}));
    h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q"}));
    let moved = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "dead"}));
    assert_eq!(moved["Messages"][0]["Body"], "long body");
}
