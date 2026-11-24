mod common;

use common::Harness;
use serde_json::{Value, json};

fn create(h: &mut Harness, name: &str, attributes: Value) {
    h.ok(json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attributes}));
}

fn send(h: &mut Harness, queue: &str, body: &str) -> Result<(), String> {
    let r = h.run(json!({"Command": "SendMessage", "QueueName": queue, "Body": body}));
    if r["ok"] == true {
        Ok(())
    } else {
        Err(r["error"]["code"].as_str().unwrap().to_owned())
    }
}

fn max_bytes(h: &mut Harness, queue: &str) -> Value {
    h.ok(json!({"Command": "GetQueueAttributes", "QueueName": queue}))["Attributes"]
        ["MaxMessageBytes"]
        .clone()
}

#[test]
fn default_and_range() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    assert_eq!(max_bytes(&mut h, "q"), 1024);
    for bad in [json!(0), json!(1025), json!("5"), json!(5.0)] {
        let code = h.err(json!({"Command": "CreateQueue", "QueueName": "x",
            "Attributes": {"MaxMessageBytes": bad}}));
        assert_eq!(code, "InvalidParameterValue");
    }
    create(&mut h, "lo", json!({"MaxMessageBytes": 1}));
    create(&mut h, "hi", json!({"MaxMessageBytes": 1024}));
    assert_eq!(max_bytes(&mut h, "lo"), 1);
}

#[test]
fn limit_counts_utf8_bytes() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({"MaxMessageBytes": 4}));
    assert_eq!(send(&mut h, "q", "abcd"), Ok(()));
    assert_eq!(send(&mut h, "q", "abcde").unwrap_err(), "InvalidParameterValue");
    assert_eq!(send(&mut h, "q", "éé"), Ok(())); // 4 bytes
    assert_eq!(send(&mut h, "q", "ééa").unwrap_err(), "InvalidParameterValue");
}

#[test]
fn rule_comes_before_other_send_rules() {
    let mut h = Harness::new();
    create(&mut h, "f.fifo", json!({"Fifo": true, "MaxMessageBytes": 2}));
    // Missing GroupId would be MissingParameter, but the size rule is first.
    let code = h.err(json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "abc"}));
    assert_eq!(code, "InvalidParameterValue");
    create(&mut h, "s", json!({"MaxMessageBytes": 2}));
    let code = h.err(json!({"Command": "SendMessage", "QueueName": "s", "Body": "abc",
        "GroupId": "g"}));
    assert_eq!(code, "InvalidParameterValue");
}

#[test]
fn set_attributes_and_existing_messages() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    send(&mut h, "q", "long body").unwrap();
    h.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q",
        "Attributes": {"MaxMessageBytes": 3}}));
    assert_eq!(max_bytes(&mut h, "q"), 3);
    let r = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q"}));
    assert_eq!(r["Messages"][0]["Body"], "long body");
    assert_eq!(send(&mut h, "q", "long").unwrap_err(), "InvalidParameterValue");
}

#[test]
fn create_rule_4_compares_it() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({"MaxMessageBytes": 10}));
    create(&mut h, "q", json!({"MaxMessageBytes": 10}));
    let code = h.err(json!({"Command": "CreateQueue", "QueueName": "q"}));
    assert_eq!(code, "QueueAlreadyExists");
}

#[test]
fn dead_letter_moves_ignore_it() {
    let mut h = Harness::new();
    create(&mut h, "dlq", json!({"MaxMessageBytes": 1}));
    create(&mut h, "q", json!({"DeadLetterQueue": "dlq", "MaxReceiveCount": 1}));
    send(&mut h, "q", "big body").unwrap();
    h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q", "VisibilityTimeout": 0}));
    h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q"}));
    let r = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "dlq"}));
    assert_eq!(r["Messages"][0]["Body"], "big body");
}
