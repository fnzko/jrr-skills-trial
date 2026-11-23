mod common;

use common::Harness;
use serde_json::{Value, json};

fn harness() -> Harness {
    let mut h = Harness::new();
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    h
}

fn tags(h: &mut Harness) -> Value {
    h.ok(json!({"Command": "ListQueueTags", "QueueName": "q"}))["Tags"].clone()
}

fn tag(h: &mut Harness, tags: Value) -> Value {
    h.run(json!({"Command": "TagQueue", "QueueName": "q", "Tags": tags}))
}

#[test]
fn tags_start_empty_and_are_replaced_by_key() {
    let mut h = harness();
    assert_eq!(tags(&mut h), json!({}));
    h.ok(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"a": "1", "b": ""}}));
    h.ok(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"a": "2", "A": "3"}}));
    assert_eq!(tags(&mut h), json!({"a": "2", "b": "", "A": "3"}));
    h.ok(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {}}));
    assert_eq!(tags(&mut h), json!({"a": "2", "b": "", "A": "3"}));
}

#[test]
fn invalid_keys_and_values_are_rejected_without_effect() {
    let mut h = harness();
    let long_key = "k".repeat(129);
    let long_value = "v".repeat(257);
    for bad in [
        json!({"ok": "1", "": "x"}),
        json!({"ok": "1", "a b": "x"}),
        json!({"ok": "1", long_key: "x"}),
        json!({"ok": "1", "k": 5}),
        json!({"ok": "1", "k": null}),
        json!({"ok": "1", "k": long_value}),
    ] {
        assert_eq!(
            h.run(json!({"Command": "TagQueue", "QueueName": "q", "Tags": bad}))["error"]["code"],
            "InvalidParameterValue",
            "{bad}"
        );
    }
    assert_eq!(tags(&mut h), json!({}));
}

#[test]
fn value_and_key_size_limits_are_inclusive() {
    let mut h = harness();
    let key = "k".repeat(128);
    // 128 characters, 256 bytes
    let value = "é".repeat(128);
    h.ok(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {key.clone(): value.clone()}}));
    assert_eq!(tags(&mut h), json!({key: value}));
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"x": "é".repeat(129)}})),
        "InvalidParameterValue"
    );
}

#[test]
fn tag_limit_is_fifty_after_the_command() {
    let mut h = harness();
    let fifty: serde_json::Map<String, Value> =
        (0..50).map(|i| (format!("k{i}"), json!("v"))).collect();
    assert_eq!(tag(&mut h, Value::Object(fifty))["ok"], true);
    // replacing an existing key does not add a tag
    assert_eq!(tag(&mut h, json!({"k0": "new"}))["ok"], true);
    assert_eq!(
        tag(&mut h, json!({"extra": "v"}))["error"]["code"],
        "LimitExceeded"
    );
    // a failed command changes nothing, even for keys that would have fit
    assert_eq!(
        tag(&mut h, json!({"k1": "changed", "extra": "v"}))["error"]["code"],
        "LimitExceeded"
    );
    let t = tags(&mut h);
    assert_eq!(t["k0"], "new");
    assert_eq!(t["k1"], "v");
    assert_eq!(t.as_object().unwrap().len(), 50);
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["k0"]}));
    assert_eq!(tag(&mut h, json!({"extra": "v"}))["ok"], true);
}

#[test]
fn untag_removes_and_ignores_unknown_keys() {
    let mut h = harness();
    tag(&mut h, json!({"a": "1", "b": "2"}));
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["a", "zz", "a"]}));
    assert_eq!(tags(&mut h), json!({"b": "2"}));
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": []}));
    assert_eq!(tags(&mut h), json!({"b": "2"}));
    for bad in [json!([1]), json!(["a b"]), json!(["a", ""]), json!("a")] {
        assert_eq!(
            h.err(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": bad})),
            "InvalidParameterValue"
        );
    }
    assert_eq!(tags(&mut h), json!({"b": "2"}));
}

#[test]
fn missing_parameters_and_unknown_queue() {
    let mut h = harness();
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "q"})),
        "MissingParameter"
    );
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "Tags": {}})),
        "MissingParameter"
    );
    assert_eq!(
        h.err(json!({"Command": "UntagQueue", "QueueName": "q"})),
        "MissingParameter"
    );
    assert_eq!(
        h.err(json!({"Command": "ListQueueTags"})),
        "MissingParameter"
    );
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "q", "Tags": []})),
        "InvalidParameterValue"
    );
    // value checks (step 3) come before the queue lookup (step 4)
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "nope", "Tags": {"a": 1}})),
        "InvalidParameterValue"
    );
    for command in [
        json!({"Command": "TagQueue", "QueueName": "nope", "Tags": {}}),
        json!({"Command": "UntagQueue", "QueueName": "nope", "TagKeys": []}),
        json!({"Command": "ListQueueTags", "QueueName": "nope"}),
    ] {
        assert_eq!(h.err(command), "QueueDoesNotExist");
    }
}

#[test]
fn tags_are_not_attributes_and_follow_purge_and_delete() {
    let mut h = harness();
    tag(&mut h, json!({"a": "1"}));
    let attrs = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert!(attrs["Attributes"].get("Tags").is_none());
    // CreateQueue rule 4 ignores tags
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    h.ok(json!({"Command": "PurgeQueue", "QueueName": "q"}));
    assert_eq!(tags(&mut h), json!({"a": "1"}));
    h.ok(json!({"Command": "DeleteQueue", "QueueName": "q"}));
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    assert_eq!(tags(&mut h), json!({}));
}
