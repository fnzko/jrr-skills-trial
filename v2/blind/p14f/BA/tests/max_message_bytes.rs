mod common;

use common::Harness;
use serde_json::json;

fn create(h: &mut Harness, name: &str, max: u64) {
    h.ok(json!({"Command": "CreateQueue", "QueueName": name, "Attributes": {"MaxMessageBytes": max}}));
}

fn send(h: &mut Harness, queue: &str, body: &str) -> serde_json::Value {
    h.run(json!({"Command": "SendMessage", "QueueName": queue, "Body": body}))
}

#[test]
fn body_longer_than_limit_is_rejected_in_utf8_bytes() {
    let mut h = Harness::new();
    create(&mut h, "q", 4);
    assert_eq!(send(&mut h, "q", "abcd")["ok"], true);
    assert_eq!(send(&mut h, "q", "abcde")["error"]["code"], "InvalidParameterValue");
    // two characters, five bytes
    assert_eq!(send(&mut h, "q", "aéééé")["error"]["code"], "InvalidParameterValue");
    assert_eq!(send(&mut h, "q", "éé")["ok"], true);
}

#[test]
fn limit_comes_before_fifo_checks() {
    let mut h = Harness::new();
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q.fifo",
        "Attributes": {"Fifo": true, "MaxMessageBytes": 1}}));
    let r = send(&mut h, "q.fifo", "ab");
    assert_eq!(r["error"]["code"], "InvalidParameterValue");
}

#[test]
fn attribute_range_and_default() {
    let mut h = Harness::new();
    for bad in [json!(0), json!(1025), json!("5"), json!(5.5)] {
        let code = h.err(json!({"Command": "CreateQueue", "QueueName": "q",
            "Attributes": {"MaxMessageBytes": bad}}));
        assert_eq!(code, "InvalidParameterValue");
    }
    create(&mut h, "q", 1);
    create(&mut h, "q", 1);
    let code = h.err(json!({"Command": "CreateQueue", "QueueName": "q",
        "Attributes": {"MaxMessageBytes": 2}}));
    assert_eq!(code, "QueueAlreadyExists");
}

#[test]
fn lowering_limit_keeps_existing_messages() {
    let mut h = Harness::new();
    create(&mut h, "q", 10);
    send(&mut h, "q", "0123456789");
    h.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q",
        "Attributes": {"MaxMessageBytes": 2}}));
    let a = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(a["Attributes"]["MaxMessageBytes"], 2);
    assert_eq!(a["Counts"]["Visible"], 1);
    let r = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q"}));
    assert_eq!(r["Messages"][0]["Body"], "0123456789");
}
