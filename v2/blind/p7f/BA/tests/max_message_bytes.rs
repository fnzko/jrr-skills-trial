mod common;

use common::Harness;
use serde_json::json;

fn create_with(h: &mut Harness, name: &str, attributes: serde_json::Value) {
    h.ok(json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attributes}));
}

fn send(h: &mut Harness, queue: &str, body: &str) -> serde_json::Value {
    h.run(json!({"Command": "SendMessage", "QueueName": queue, "Body": body}))
}

#[test]
fn attribute_range_is_one_to_1024() {
    let mut h = Harness::new();
    for bad in [json!(0), json!(1025), json!("5"), json!(5.5)] {
        let c = json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": {"MaxMessageBytes": bad}});
        assert_eq!(h.err(c), "InvalidParameterValue", "{bad}");
    }
    create_with(&mut h, "lo", json!({"MaxMessageBytes": 1}));
    create_with(&mut h, "hi", json!({"MaxMessageBytes": 1024}));
    let r = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "lo"}));
    assert_eq!(r["Attributes"]["MaxMessageBytes"], 1);
}

#[test]
fn body_limit_counts_utf8_bytes() {
    let mut h = Harness::new();
    create_with(&mut h, "q", json!({"MaxMessageBytes": 4}));
    assert_eq!(send(&mut h, "q", "abcd")["ok"], true);
    // two characters, four bytes
    assert_eq!(send(&mut h, "q", "éé")["ok"], true);
    // two characters, five bytes
    assert_eq!(
        send(&mut h, "q", "éé!")["error"]["code"],
        "InvalidParameterValue"
    );
    assert_eq!(
        send(&mut h, "q", "abcde")["error"]["code"],
        "InvalidParameterValue"
    );
}

#[test]
fn limit_rule_comes_before_queue_type_rules() {
    let mut h = Harness::new();
    create_with(&mut h, "q", json!({"MaxMessageBytes": 2}));
    let c = json!({"Command": "SendMessage", "QueueName": "q", "Body": "abc", "GroupId": "g"});
    assert_eq!(h.err(c), "InvalidParameterValue");
    create_with(
        &mut h,
        "f.fifo",
        json!({"Fifo": true, "MaxMessageBytes": 2}),
    );
    // without the limit rule this would be a MissingParameter for GroupId
    assert_eq!(
        send(&mut h, "f.fifo", "abc")["error"]["code"],
        "InvalidParameterValue"
    );
}

#[test]
fn set_attributes_changes_limit_but_keeps_old_messages() {
    let mut h = Harness::new();
    create_with(&mut h, "q", json!({}));
    assert_eq!(send(&mut h, "q", "long body")["ok"], true);
    h.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"MaxMessageBytes": 3}}));
    assert_eq!(
        send(&mut h, "q", "long body")["error"]["code"],
        "InvalidParameterValue"
    );
    let r = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q"}));
    assert_eq!(r["Messages"][0]["Body"], "long body");
}

#[test]
fn create_again_compares_max_message_bytes() {
    let mut h = Harness::new();
    create_with(&mut h, "q", json!({"MaxMessageBytes": 10}));
    create_with(&mut h, "q", json!({"MaxMessageBytes": 10}));
    let c = json!({"Command": "CreateQueue", "QueueName": "q"});
    assert_eq!(h.err(c), "QueueAlreadyExists");
}

#[test]
fn dead_letter_move_ignores_limit() {
    let mut h = Harness::new();
    create_with(&mut h, "dlq", json!({"MaxMessageBytes": 1}));
    create_with(
        &mut h,
        "q",
        json!({"DeadLetterQueue": "dlq", "MaxReceiveCount": 1}),
    );
    assert_eq!(send(&mut h, "q", "long body")["ok"], true);
    h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q", "VisibilityTimeout": 0}));
    let r = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q"}));
    assert_eq!(r["Messages"], json!([]));
    let r = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "dlq"}));
    assert_eq!(r["Messages"][0]["Body"], "long body");
}
