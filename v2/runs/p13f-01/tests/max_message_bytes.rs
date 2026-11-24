mod common;

use common::Harness;
use serde_json::json;

fn create_with_limit(h: &mut Harness, name: &str, limit: u64) {
    h.ok(json!({
        "Command": "CreateQueue", "QueueName": name,
        "Attributes": {"MaxMessageBytes": limit},
    }));
}

fn send(name: &str, body: &str) -> serde_json::Value {
    json!({"Command": "SendMessage", "QueueName": name, "Body": body})
}

#[test]
fn body_longer_than_limit_is_rejected_and_equal_is_accepted() {
    let mut h = Harness::new();
    create_with_limit(&mut h, "q", 4);
    assert_eq!(h.err(send("q", "abcde")), "InvalidParameterValue");
    h.ok(send("q", "abcd"));
}

#[test]
fn limit_counts_utf8_bytes() {
    let mut h = Harness::new();
    create_with_limit(&mut h, "q", 3);
    assert_eq!(h.err(send("q", "éé")), "InvalidParameterValue");
    h.ok(send("q", "é"));
}

#[test]
fn limit_comes_before_standard_queue_unsupported_check() {
    let mut h = Harness::new();
    create_with_limit(&mut h, "q", 1);
    let mut command = send("q", "ab");
    command["GroupId"] = json!("g");
    assert_eq!(h.err(command), "InvalidParameterValue");
}

#[test]
fn limit_comes_after_queue_lookup() {
    let mut h = Harness::new();
    assert_eq!(h.err(send("missing", "ab")), "QueueDoesNotExist");
}

#[test]
fn range_is_one_to_1024_and_default_is_1024() {
    let mut h = Harness::new();
    for bad in [0, 1025] {
        assert_eq!(
            h.err(json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": {"MaxMessageBytes": bad}})),
            "InvalidParameterValue"
        );
    }
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    let r = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(r["Attributes"]["MaxMessageBytes"], 1024);
}

#[test]
fn recreate_with_different_limit_conflicts() {
    let mut h = Harness::new();
    create_with_limit(&mut h, "q", 10);
    create_with_limit(&mut h, "q", 10);
    assert_eq!(
        h.err(json!({"Command": "CreateQueue", "QueueName": "q"})),
        "QueueAlreadyExists"
    );
}

#[test]
fn lowering_limit_keeps_existing_messages() {
    let mut h = Harness::new();
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    h.ok(send("q", "long body"));
    h.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"MaxMessageBytes": 2}}));
    let r = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(r["Attributes"]["MaxMessageBytes"], 2);
    assert_eq!(r["Counts"]["Visible"], 1);
    let r = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q"}));
    assert_eq!(r["Messages"][0]["Body"], "long body");
}

#[test]
fn dead_letter_move_ignores_limit() {
    let mut h = Harness::new();
    create_with_limit(&mut h, "dead", 1);
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": {"DeadLetterQueue": "dead", "MaxReceiveCount": 1, "VisibilityTimeout": 0}}));
    h.ok(send("q", "long body"));
    h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q"}));
    h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q"}));
    let r = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "dead"}));
    assert_eq!(r["Messages"][0]["Body"], "long body");
}
