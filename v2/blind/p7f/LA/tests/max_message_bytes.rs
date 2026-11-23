mod common;

use common::Harness;
use serde_json::json;

fn create(h: &mut Harness, name: &str, attributes: serde_json::Value) {
    h.ok(json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attributes}));
}

fn send(h: &mut Harness, body: &str) -> serde_json::Value {
    h.run(json!({"Command": "SendMessage", "QueueName": "q", "Body": body}))
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
    create(&mut h, "q", json!({"MaxMessageBytes": null}));
    let r = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(r["Attributes"]["MaxMessageBytes"], 1024);
    // Same attributes after defaults: idempotent; different: conflict.
    create(&mut h, "q", json!({}));
    assert_eq!(
        h.err(json!({"Command": "CreateQueue", "QueueName": "q",
                      "Attributes": {"MaxMessageBytes": 10}})),
        "QueueAlreadyExists"
    );
}

#[test]
fn send_limit_counts_utf8_bytes() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({"MaxMessageBytes": 4}));
    assert_eq!(send(&mut h, "abcd")["ok"], true);
    assert_eq!(send(&mut h, "é é")["ok"], false); // 5 bytes
    assert_eq!(send(&mut h, "éé")["ok"], true); // 4 bytes
    let r = send(&mut h, "abcde");
    assert_eq!(r["error"]["code"], "InvalidParameterValue");
}

#[test]
fn limit_comes_before_fifo_rules() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({"MaxMessageBytes": 2}));
    let r = h.run(
        json!({"Command": "SendMessage", "QueueName": "q", "Body": "abc",
                          "GroupId": "g"}),
    );
    assert_eq!(r["error"]["code"], "InvalidParameterValue");
}

#[test]
fn changing_limit_keeps_existing_messages() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    assert_eq!(send(&mut h, "abcdef")["ok"], true);
    h.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q",
                 "Attributes": {"MaxMessageBytes": 3}}));
    let r = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q"}));
    assert_eq!(r["Messages"][0]["Body"], "abcdef");
    assert_eq!(send(&mut h, "abcd")["ok"], false);
}

#[test]
fn dead_letter_moves_ignore_limit() {
    let mut h = Harness::new();
    create(&mut h, "dlq", json!({"MaxMessageBytes": 1}));
    create(
        &mut h,
        "q",
        json!({"DeadLetterQueue": "dlq", "MaxReceiveCount": 1,
                                "VisibilityTimeout": 0}),
    );
    assert_eq!(send(&mut h, "abcdef")["ok"], true);
    h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q"}));
    h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q"}));
    let r = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "dlq"}));
    assert_eq!(r["Messages"][0]["Body"], "abcdef");
}
