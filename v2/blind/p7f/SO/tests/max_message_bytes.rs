mod common;

use common::Harness;
use serde_json::json;

fn create_with(h: &mut Harness, name: &str, attributes: serde_json::Value) {
    h.ok(json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attributes}));
}

fn send(h: &mut Harness, queue: &str, body: &str) -> String {
    h.err(json!({"Command": "SendMessage", "QueueName": queue, "Body": body}))
}

#[test]
fn limit_counts_utf8_bytes() {
    let mut h = Harness::new();
    create_with(&mut h, "q", json!({"MaxMessageBytes": 4}));
    h.ok(json!({"Command": "SendMessage", "QueueName": "q", "Body": "abcd"}));
    h.ok(json!({"Command": "SendMessage", "QueueName": "q", "Body": "éé"}));
    assert_eq!(send(&mut h, "q", "abcde"), "InvalidParameterValue");
    assert_eq!(send(&mut h, "q", "éé€"), "InvalidParameterValue");
}

#[test]
fn limit_is_checked_before_fifo_rules() {
    let mut h = Harness::new();
    create_with(
        &mut h,
        "q.fifo",
        json!({"Fifo": true, "MaxMessageBytes": 1}),
    );
    // Missing GroupId would be MissingParameter, but the size rule comes first.
    assert_eq!(send(&mut h, "q.fifo", "ab"), "InvalidParameterValue");
    let mut h = Harness::new();
    create_with(&mut h, "q", json!({"MaxMessageBytes": 1}));
    let code =
        h.err(json!({"Command": "SendMessage", "QueueName": "q", "Body": "ab", "GroupId": "g"}));
    assert_eq!(code, "InvalidParameterValue");
}

#[test]
fn attribute_range_and_default() {
    let mut h = Harness::new();
    for bad in [json!(0), json!(1025), json!("5"), json!(5.5)] {
        let code = h.err(json!({"Command": "CreateQueue", "QueueName": "q",
            "Attributes": {"MaxMessageBytes": bad}}));
        assert_eq!(code, "InvalidParameterValue");
    }
    create_with(&mut h, "q", json!({"MaxMessageBytes": null}));
    let r = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(r["Attributes"]["MaxMessageBytes"], 1024);
}

#[test]
fn takes_part_in_create_rule_4() {
    let mut h = Harness::new();
    create_with(&mut h, "q", json!({"MaxMessageBytes": 10}));
    create_with(&mut h, "q", json!({"MaxMessageBytes": 10}));
    let code = h.err(json!({"Command": "CreateQueue", "QueueName": "q"}));
    assert_eq!(code, "QueueAlreadyExists");
}

#[test]
fn changing_it_keeps_existing_messages() {
    let mut h = Harness::new();
    create_with(&mut h, "q", json!({}));
    h.ok(json!({"Command": "SendMessage", "QueueName": "q", "Body": "long body"}));
    h.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q",
        "Attributes": {"MaxMessageBytes": 2}}));
    assert_eq!(send(&mut h, "q", "long body"), "InvalidParameterValue");
    let r = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q"}));
    assert_eq!(r["Messages"][0]["Body"], "long body");
}

#[test]
fn dead_letter_moves_ignore_it() {
    let mut h = Harness::new();
    create_with(&mut h, "dlq", json!({"MaxMessageBytes": 1}));
    create_with(
        &mut h,
        "q",
        json!({"DeadLetterQueue": "dlq", "MaxReceiveCount": 1,
        "VisibilityTimeout": 0}),
    );
    h.ok(json!({"Command": "SendMessage", "QueueName": "q", "Body": "too long for dlq"}));
    h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q"}));
    let r = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q"}));
    assert_eq!(r["Messages"], json!([]));
    let r = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "dlq"}));
    assert_eq!(r["Messages"][0]["Body"], "too long for dlq");
}
