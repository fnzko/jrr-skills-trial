mod common;

use common::Harness;
use serde_json::json;

fn create(h: &mut Harness, name: &str, attributes: serde_json::Value) {
    h.ok(json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attributes}));
}

fn send(h: &mut Harness, queue: &str, body: &str) -> serde_json::Value {
    h.run(json!({"Command": "SendMessage", "QueueName": queue, "Body": body}))
}

#[test]
fn default_is_1024_and_is_reported() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let r = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(r["Attributes"]["MaxMessageBytes"], 1024);
}

#[test]
fn range_is_1_to_1024() {
    let mut h = Harness::new();
    for bad in [json!(0), json!(1025), json!("5"), json!(5.0)] {
        assert_eq!(
            h.err(json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": {"MaxMessageBytes": bad}})),
            "InvalidParameterValue"
        );
    }
    create(&mut h, "q", json!({"MaxMessageBytes": 1}));
    create(&mut h, "r", json!({"MaxMessageBytes": 1024}));
}

#[test]
fn send_over_the_limit_counts_utf8_bytes() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({"MaxMessageBytes": 4}));
    assert_eq!(send(&mut h, "q", "abcd")["ok"], true);
    assert_eq!(send(&mut h, "q", "abcde")["error"]["code"], "InvalidParameterValue");
    // two characters, four bytes: fits; three characters, six bytes: does not
    assert_eq!(send(&mut h, "q", "éé")["ok"], true);
    assert_eq!(send(&mut h, "q", "ééé")["error"]["code"], "InvalidParameterValue");
    // failed sends use no message id
    assert_eq!(send(&mut h, "q", "a")["MessageId"], "m-3");
}

#[test]
fn limit_rule_comes_before_the_queue_type_rules() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({"MaxMessageBytes": 2}));
    let r = h.run(json!({"Command": "SendMessage", "QueueName": "q", "Body": "abc", "GroupId": "g"}));
    assert_eq!(r["error"]["code"], "InvalidParameterValue");
    create(&mut h, "f.fifo", json!({"Fifo": true, "MaxMessageBytes": 2}));
    let r = h.run(json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "abc"}));
    assert_eq!(r["error"]["code"], "InvalidParameterValue");
}

#[test]
fn body_rule_of_1024_bytes_stays() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    assert_eq!(send(&mut h, "q", &"a".repeat(1025))["error"]["code"], "InvalidParameterValue");
    assert_eq!(send(&mut h, "q", "")["error"]["code"], "InvalidParameterValue");
    assert_eq!(send(&mut h, "q", &"a".repeat(1024))["ok"], true);
}

#[test]
fn set_attributes_does_not_touch_existing_messages() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    assert_eq!(send(&mut h, "q", "longer body")["ok"], true);
    h.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"MaxMessageBytes": 3}}));
    assert_eq!(send(&mut h, "q", "longer body")["error"]["code"], "InvalidParameterValue");
    let r = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q"}));
    assert_eq!(r["Messages"][0]["Body"], "longer body");
}

#[test]
fn create_again_compares_max_message_bytes() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({"MaxMessageBytes": 10}));
    create(&mut h, "q", json!({"MaxMessageBytes": 10}));
    assert_eq!(
        h.err(json!({"Command": "CreateQueue", "QueueName": "q"})),
        "QueueAlreadyExists"
    );
}

#[test]
fn dead_letter_moves_ignore_the_limit() {
    let mut h = Harness::new();
    create(&mut h, "dlq", json!({"MaxMessageBytes": 1}));
    create(&mut h, "src", json!({"DeadLetterQueue": "dlq", "MaxReceiveCount": 1}));
    assert_eq!(send(&mut h, "src", "long body")["ok"], true);
    h.ok(json!({"Command": "ReceiveMessage", "QueueName": "src", "VisibilityTimeout": 0}));
    h.ok(json!({"Command": "ReceiveMessage", "QueueName": "src"}));
    let r = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "dlq"}));
    assert_eq!(r["Messages"][0]["Body"], "long body");
}
