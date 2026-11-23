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

fn tags(h: &mut Harness) -> Value {
    h.ok(json!({"Command": "ListQueueTags", "QueueName": "q"}))["Tags"].clone()
}

#[test]
fn new_queue_has_no_tags() {
    let mut h = harness();
    assert_eq!(tags(&mut h), json!({}));
}

#[test]
fn tag_sets_and_replaces() {
    let mut h = harness();
    h.ok(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"a": "1", "b": ""}}));
    h.ok(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"a": "2", "A": "x"}}));
    assert_eq!(tags(&mut h), json!({"a": "2", "b": "", "A": "x"}));
    h.ok(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {}}));
    assert_eq!(tags(&mut h), json!({"a": "2", "b": "", "A": "x"}));
}

#[test]
fn untag_removes_and_ignores_unknown_keys() {
    let mut h = harness();
    tag(&mut h, json!({"a": "1", "b": "2"}));
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["a", "zzz"]}));
    assert_eq!(tags(&mut h), json!({"b": "2"}));
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": []}));
    assert_eq!(tags(&mut h), json!({"b": "2"}));
}

#[test]
fn invalid_tag_input() {
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
    ] {
        assert_eq!(
            h.err(json!({"Command": "TagQueue", "QueueName": "q", "Tags": bad})),
            "InvalidParameterValue"
        );
    }
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "q", "Tags": []})),
        "InvalidParameterValue"
    );
    for bad in [json!([1]), json!(["a b"]), json!("a"), json!([""])] {
        assert_eq!(
            h.err(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": bad})),
            "InvalidParameterValue"
        );
    }
    assert_eq!(tags(&mut h), json!({}));
}

#[test]
fn tag_limits_on_key_and_value_sizes_are_inclusive() {
    let mut h = harness();
    let key = "k".repeat(128);
    let value = "é".repeat(128);
    let response = tag(&mut h, json!({key.clone(): value.clone()}));
    assert_eq!(response["ok"], true);
    assert_eq!(tags(&mut h), json!({key: value}));
}

#[test]
fn at_most_fifty_tags() {
    let mut h = harness();
    let fifty: serde_json::Map<String, Value> =
        (0..50).map(|i| (format!("k{i}"), json!("v"))).collect();
    assert_eq!(tag(&mut h, Value::Object(fifty))["ok"], true);
    // replacing an existing key does not add a tag
    assert_eq!(tag(&mut h, json!({"k0": "new"}))["ok"], true);
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"extra": "v"}})),
        "LimitExceeded"
    );
    // a failed command changes nothing, even with a replaced key beside it
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"k1": "z", "extra": "v"}})),
        "LimitExceeded"
    );
    let listed = tags(&mut h);
    assert_eq!(listed.as_object().unwrap().len(), 50);
    assert_eq!(listed["k0"], "new");
    assert_eq!(listed["k1"], "v");
}

#[test]
fn check_order_for_tag_commands() {
    let mut h = harness();
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "q"})),
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
        h.err(json!({"Command": "TagQueue", "QueueName": "nope", "Tags": {"a": 1}})),
        "InvalidParameterValue"
    );
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "nope", "Tags": {}})),
        "QueueDoesNotExist"
    );
    assert_eq!(
        h.err(json!({"Command": "UntagQueue", "QueueName": "nope", "TagKeys": []})),
        "QueueDoesNotExist"
    );
    assert_eq!(
        h.err(json!({"Command": "ListQueueTags", "QueueName": "nope"})),
        "QueueDoesNotExist"
    );
}

#[test]
fn tags_are_not_attributes_and_survive_purge_but_not_delete() {
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
