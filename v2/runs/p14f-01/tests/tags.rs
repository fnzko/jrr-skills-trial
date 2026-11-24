mod common;

use common::Harness;
use serde_json::{Value, json};

fn harness() -> Harness {
    let mut h = Harness::new();
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    h
}

fn tag(h: &mut Harness, tags: Value) -> Value {
    h.run(json!({"Command": "TagQueue", "QueueName": "q", "Tags": tags}))
}

fn listed(h: &mut Harness) -> Value {
    h.ok(json!({"Command": "ListQueueTags", "QueueName": "q"}))["Tags"].clone()
}

#[test]
fn new_queue_has_no_tags_and_set_replaces() {
    let mut h = harness();
    assert_eq!(listed(&mut h), json!({}));
    assert_eq!(tag(&mut h, json!({"a": "1", "B": ""}))["ok"], true);
    assert_eq!(tag(&mut h, json!({"a": "2"}))["ok"], true);
    assert_eq!(listed(&mut h), json!({"a": "2", "B": ""}));
    assert_eq!(tag(&mut h, json!({}))["ok"], true);
    assert_eq!(listed(&mut h), json!({"a": "2", "B": ""}));
}

#[test]
fn invalid_keys_and_values() {
    let mut h = harness();
    let long_key = "k".repeat(129);
    let long_value = "v".repeat(257);
    for bad in [
        json!({"": "x"}),
        json!({"a b": "x"}),
        json!({long_key: "x"}),
        json!({"a": 1}),
        json!({"a": null}),
        json!({"a": long_value}),
        json!({"a": "é".repeat(129)}),
        json!([]),
    ] {
        let r = tag(&mut h, bad);
        assert_eq!(r["error"]["code"], "InvalidParameterValue", "{r}");
    }
    assert_eq!(listed(&mut h), json!({}));
    let edge = json!({"k".repeat(128): "é".repeat(128)});
    assert_eq!(tag(&mut h, edge)["ok"], true);
}

#[test]
fn fifty_tags_fit_and_the_fifty_first_fails_atomically() {
    let mut h = harness();
    let fifty: serde_json::Map<String, Value> =
        (0..50).map(|i| (format!("k{i}"), json!("v"))).collect();
    assert_eq!(tag(&mut h, Value::Object(fifty))["ok"], true);
    let r = tag(&mut h, json!({"k0": "changed", "extra": "x"}));
    assert_eq!(r["error"]["code"], "LimitExceeded");
    assert_eq!(listed(&mut h)["k0"], "v");
    assert_eq!(listed(&mut h).as_object().unwrap().len(), 50);
    assert_eq!(tag(&mut h, json!({"k0": "changed"}))["ok"], true);
    assert_eq!(listed(&mut h)["k0"], "changed");
}

#[test]
fn untag_removes_given_keys_and_ignores_unknown() {
    let mut h = harness();
    tag(&mut h, json!({"a": "1", "b": "2", "c": "3"}));
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["a", "zzz", "A"]}));
    assert_eq!(listed(&mut h), json!({"b": "2", "c": "3"}));
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": []}));
    assert_eq!(listed(&mut h), json!({"b": "2", "c": "3"}));
}

#[test]
fn untag_rejects_bad_elements_without_removing_anything() {
    let mut h = harness();
    tag(&mut h, json!({"a": "1"}));
    for bad in [json!(["a", 1]), json!(["a", "b c"]), json!(["a", ""]), json!("a")] {
        let code = h.err(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": bad}));
        assert_eq!(code, "InvalidParameterValue");
    }
    assert_eq!(listed(&mut h), json!({"a": "1"}));
}

#[test]
fn error_order_for_tag_commands() {
    let mut h = harness();
    for command in ["TagQueue", "UntagQueue", "ListQueueTags"] {
        assert_eq!(h.err(json!({"Command": command})), "MissingParameter");
        assert_eq!(
            h.err(json!({"Command": command, "QueueName": "nope", "Tags": {}, "TagKeys": []})),
            "QueueDoesNotExist"
        );
    }
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "q"})),
        "MissingParameter"
    );
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "nope", "Tags": {"a b": "x"}})),
        "InvalidParameterValue"
    );
}

#[test]
fn tags_are_not_attributes_and_survive_purge_but_not_delete() {
    let mut h = harness();
    tag(&mut h, json!({"a": "1"}));
    let attrs = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(
        attrs["Attributes"],
        json!({
            "VisibilityTimeout": 30, "DelaySeconds": 0, "RetentionSeconds": 345600,
            "Fifo": false, "ContentBasedDeduplication": false, "MaxMessageBytes": 1024,
        })
    );
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    h.ok(json!({"Command": "PurgeQueue", "QueueName": "q"}));
    assert_eq!(listed(&mut h), json!({"a": "1"}));
    h.ok(json!({"Command": "DeleteQueue", "QueueName": "q"}));
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    assert_eq!(listed(&mut h), json!({}));
}
