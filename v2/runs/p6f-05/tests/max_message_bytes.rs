mod common;

use common::Harness;
use serde_json::json;

fn send(h: &mut Harness, queue: &str, body: &str) -> serde_json::Value {
    h.run(json!({"Command": "SendMessage", "QueueName": queue, "Body": body}))
}

#[test]
fn limit_counts_utf8_bytes_not_characters() {
    let mut h = Harness::new();
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": {"MaxMessageBytes": 4}}));
    assert_eq!(send(&mut h, "q", "abcd")["ok"], true);
    assert_eq!(send(&mut h, "q", "ab\u{e9}")["ok"], true);
    assert_eq!(send(&mut h, "q", "abc\u{e9}")["error"]["code"], "InvalidParameterValue");
    assert_eq!(send(&mut h, "q", "abcde")["error"]["code"], "InvalidParameterValue");
}

#[test]
fn attribute_range_and_default() {
    let mut h = Harness::new();
    for bad in [json!(0), json!(1025), json!("5"), json!(5.5)] {
        let c = json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": {"MaxMessageBytes": bad}});
        assert_eq!(h.err(c), "InvalidParameterValue", "{bad}");
    }
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    let r = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(r["Attributes"]["MaxMessageBytes"], 1024);
    // Rule 4 of CreateQueue compares the new attribute too.
    let same = json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": {"MaxMessageBytes": 1024}});
    h.ok(same);
    let other = json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": {"MaxMessageBytes": 10}});
    assert_eq!(h.err(other), "QueueAlreadyExists");
}

#[test]
fn lowering_the_limit_keeps_existing_messages() {
    let mut h = Harness::new();
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    send(&mut h, "q", "long enough body");
    h.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"MaxMessageBytes": 3}}));
    let r = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q"}));
    assert_eq!(r["Messages"][0]["Body"], "long enough body");
    assert_eq!(send(&mut h, "q", "four")["error"]["code"], "InvalidParameterValue");
}

#[test]
fn size_rule_comes_before_fifo_rules() {
    let mut h = Harness::new();
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q.fifo",
        "Attributes": {"Fifo": true, "MaxMessageBytes": 2}}));
    // Without GroupId this would be MissingParameter, and with DelaySeconds UnsupportedOperation.
    assert_eq!(send(&mut h, "q.fifo", "abc")["error"]["code"], "InvalidParameterValue");
    let c = json!({"Command": "SendMessage", "QueueName": "q.fifo", "Body": "abc", "DelaySeconds": 1});
    assert_eq!(h.err(c), "InvalidParameterValue");
}

#[test]
fn body_rule_of_1024_bytes_stays_a_parameter_check() {
    let mut h = Harness::new();
    let c = json!({"Command": "SendMessage", "QueueName": "missing", "Body": "x".repeat(1025)});
    assert_eq!(h.err(c), "InvalidParameterValue");
}
