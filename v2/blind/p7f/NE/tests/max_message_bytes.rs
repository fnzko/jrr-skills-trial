mod common;

use common::Harness;
use serde_json::json;

fn create(h: &mut Harness, name: &str, attributes: serde_json::Value) {
    h.ok(json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attributes}));
}

fn send(h: &mut Harness, queue: &str, body: &str) -> String {
    let response = h.run(json!({"Command": "SendMessage", "QueueName": queue, "Body": body}));
    if response["ok"] == true {
        "ok".to_owned()
    } else {
        response["error"]["code"].as_str().unwrap().to_owned()
    }
}

#[test]
fn limit_is_in_utf8_bytes() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({"MaxMessageBytes": 4}));
    assert_eq!(send(&mut h, "q", "abcd"), "ok");
    assert_eq!(send(&mut h, "q", "abcde"), "InvalidParameterValue");
    // two characters, four bytes
    assert_eq!(send(&mut h, "q", "éé"), "ok");
    // two characters, five bytes
    assert_eq!(send(&mut h, "q", "ééa"), "InvalidParameterValue");
}

#[test]
fn default_is_1024_and_is_reported() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let r = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(r["Attributes"]["MaxMessageBytes"], 1024);
    assert_eq!(send(&mut h, "q", &"a".repeat(1024)), "ok");
    assert_eq!(send(&mut h, "q", &"a".repeat(1025)), "InvalidParameterValue");
}

#[test]
fn attribute_range_is_1_to_1024() {
    let mut h = Harness::new();
    for bad in [json!(0), json!(1025), json!("5"), json!(5.5)] {
        assert_eq!(
            h.err(json!({"Command": "CreateQueue", "QueueName": "q",
                         "Attributes": {"MaxMessageBytes": bad}})),
            "InvalidParameterValue"
        );
    }
    create(&mut h, "q", json!({"MaxMessageBytes": 1}));
    assert_eq!(send(&mut h, "q", "a"), "ok");
    assert_eq!(send(&mut h, "q", "ab"), "InvalidParameterValue");
}

#[test]
fn create_again_compares_it() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({"MaxMessageBytes": 10}));
    create(&mut h, "q", json!({"MaxMessageBytes": 10}));
    assert_eq!(
        h.err(json!({"Command": "CreateQueue", "QueueName": "q"})),
        "QueueAlreadyExists"
    );
}

#[test]
fn set_applies_to_later_sends_only() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    assert_eq!(send(&mut h, "q", "abcdef"), "ok");
    h.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q",
                "Attributes": {"MaxMessageBytes": 3}}));
    assert_eq!(send(&mut h, "q", "abcd"), "InvalidParameterValue");
    let received = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q"}));
    assert_eq!(received["Messages"][0]["Body"], "abcdef");
}

#[test]
fn size_rule_comes_before_fifo_rules_but_after_body_range() {
    let mut h = Harness::new();
    create(&mut h, "s", json!({"MaxMessageBytes": 2}));
    create(&mut h, "f.fifo", json!({"Fifo": true, "MaxMessageBytes": 2}));
    // rule 0 beats rule 1 (standard queue with GroupId)
    assert_eq!(
        h.err(json!({"Command": "SendMessage", "QueueName": "s", "Body": "abc", "GroupId": "g"})),
        "InvalidParameterValue"
    );
    // rule 0 beats rule 2 (missing GroupId)
    assert_eq!(
        h.err(json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "abc"})),
        "InvalidParameterValue"
    );
    // a missing queue is still checked first
    assert_eq!(
        h.err(json!({"Command": "SendMessage", "QueueName": "nope", "Body": "abc"})),
        "QueueDoesNotExist"
    );
}

#[test]
fn dead_letter_moves_ignore_the_limit() {
    let mut h = Harness::new();
    create(&mut h, "dlq", json!({"MaxMessageBytes": 1}));
    create(&mut h, "q", json!({"DeadLetterQueue": "dlq", "MaxReceiveCount": 1, "VisibilityTimeout": 0}));
    assert_eq!(send(&mut h, "q", "long body"), "ok");
    h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q"}));
    let second = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q"}));
    assert_eq!(second["Messages"], json!([]));
    let moved = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "dlq"}));
    assert_eq!(moved["Messages"][0]["Body"], "long body");
}
