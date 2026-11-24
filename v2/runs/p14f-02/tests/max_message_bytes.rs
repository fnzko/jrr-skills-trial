mod common;

use common::Harness;
use serde_json::json;

fn create(h: &mut Harness, name: &str, attributes: serde_json::Value) {
    h.ok(json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attributes}));
}

fn send(body: &str) -> serde_json::Value {
    json!({"Command": "SendMessage", "QueueName": "q", "Body": body})
}

#[test]
fn body_over_the_limit_is_rejected_and_at_the_limit_accepted() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({"MaxMessageBytes": 4}));
    assert_eq!(h.err(send("abcde")), "InvalidParameterValue");
    h.ok(send("abcd"));
}

#[test]
fn limit_counts_utf8_bytes() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({"MaxMessageBytes": 3}));
    assert_eq!(h.err(send("éé")), "InvalidParameterValue");
    h.ok(send("é"));
}

#[test]
fn limit_rule_precedes_unsupported_operation() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({"MaxMessageBytes": 1}));
    let mut command = send("ab");
    command["GroupId"] = json!("g");
    assert_eq!(h.err(command), "InvalidParameterValue");
}

#[test]
fn attribute_defaults_ranges_and_update() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let get = |h: &mut Harness| {
        h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}))["Attributes"]
            ["MaxMessageBytes"]
            .clone()
    };
    assert_eq!(get(&mut h), json!(1024));
    for bad in [json!(0), json!(1025), json!("5"), json!(5.5)] {
        let set = json!({"Command": "SetQueueAttributes", "QueueName": "q",
                         "Attributes": {"MaxMessageBytes": bad}});
        assert_eq!(h.err(set), "InvalidParameterValue", "{bad}");
    }
    h.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q",
                "Attributes": {"MaxMessageBytes": 1}}));
    assert_eq!(get(&mut h), json!(1));
}

#[test]
fn lowering_the_limit_keeps_queued_messages() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    h.ok(send("abcdef"));
    h.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q",
                "Attributes": {"MaxMessageBytes": 2}}));
    let r = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q"}));
    assert_eq!(r["Messages"][0]["Body"], "abcdef");
}

#[test]
fn create_again_with_a_different_limit_conflicts() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({"MaxMessageBytes": 10}));
    create(&mut h, "q", json!({"MaxMessageBytes": 10}));
    let again = json!({"Command": "CreateQueue", "QueueName": "q"});
    assert_eq!(h.err(again), "QueueAlreadyExists");
}
