mod common;

use common::Harness;
use serde_json::{Value, json};

fn harness_with_queue() -> Harness {
    let mut h = Harness::new();
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    h
}

fn tag(h: &mut Harness, tags: Value) -> Value {
    h.run(json!({"Command": "TagQueue", "QueueName": "q", "Tags": tags}))
}

fn list(h: &mut Harness) -> Value {
    h.ok(json!({"Command": "ListQueueTags", "QueueName": "q"}))["Tags"].clone()
}

#[test]
fn new_queue_has_no_tags() {
    let mut h = harness_with_queue();
    assert_eq!(list(&mut h), json!({}));
}

#[test]
fn tag_sets_and_replaces_values() {
    let mut h = harness_with_queue();
    h.ok(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"a": "1", "B": ""}}));
    h.ok(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"a": "2", "b": "x"}}));
    assert_eq!(list(&mut h), json!({"a": "2", "B": "", "b": "x"}));
}

#[test]
fn untag_removes_keys_and_ignores_unknown_ones() {
    let mut h = harness_with_queue();
    h.ok(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"a": "1", "b": "2"}}));
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["a", "zzz"]}));
    assert_eq!(list(&mut h), json!({"b": "2"}));
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": []}));
    h.ok(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {}}));
    assert_eq!(list(&mut h), json!({"b": "2"}));
}

#[test]
fn invalid_keys_and_values() {
    let mut h = harness_with_queue();
    let long_key = "k".repeat(129);
    let long_value = "v".repeat(257);
    let bad = [
        json!({"": "v"}),
        json!({"a b": "v"}),
        json!({long_key: "v"}),
        json!({"k": 1}),
        json!({"k": null}),
        json!({"k": long_value}),
        json!({"ok": "fine", "k": ["x"]}),
    ];
    for tags in bad {
        let response = tag(&mut h, tags.clone());
        assert_eq!(response["error"]["code"], "InvalidParameterValue", "{tags}");
    }
    assert_eq!(list(&mut h), json!({}));
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "q", "Tags": []})),
        "InvalidParameterValue"
    );
    for keys in [json!(["a", 1]), json!(["a b"]), json!([""]), json!("a")] {
        assert_eq!(
            h.err(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": keys})),
            "InvalidParameterValue",
            "{keys}"
        );
    }
}

#[test]
fn value_limit_is_in_utf8_bytes() {
    let mut h = harness_with_queue();
    // 128 two-byte characters: exactly 256 bytes
    h.ok(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"a": "\u{e9}".repeat(128)}}));
    assert_eq!(
        tag(&mut h, json!({"b": "\u{e9}".repeat(129)}))["error"]["code"],
        "InvalidParameterValue"
    );
}

#[test]
fn key_limits_are_inclusive() {
    let mut h = harness_with_queue();
    h.ok(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"k".repeat(128): "v"}}));
    assert_eq!(list(&mut h).as_object().unwrap().len(), 1);
}

#[test]
fn at_most_fifty_tags() {
    let mut h = harness_with_queue();
    let fifty: serde_json::Map<String, Value> =
        (0..50).map(|i| (format!("k{i}"), json!("v"))).collect();
    h.ok(json!({"Command": "TagQueue", "QueueName": "q", "Tags": fifty}));
    // replacing an existing key is not growth
    h.ok(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"k0": "new"}}));
    let response = tag(&mut h, json!({"k0": "again", "extra": "v"}));
    assert_eq!(response["error"]["code"], "LimitExceeded");
    // a failed command changes nothing
    assert_eq!(list(&mut h)["k0"], "new");
    assert_eq!(list(&mut h).as_object().unwrap().len(), 50);
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["k1"]}));
    h.ok(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"extra": "v"}}));
}

#[test]
fn check_order() {
    let mut h = harness_with_queue();
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "q"})),
        "MissingParameter"
    );
    assert_eq!(
        h.err(json!({"Command": "UntagQueue", "TagKeys": []})),
        "MissingParameter"
    );
    assert_eq!(
        h.err(json!({"Command": "ListQueueTags"})),
        "MissingParameter"
    );
    // values are checked before the queue lookup
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "nope", "Tags": {"a b": "v"}})),
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
fn tags_are_not_attributes_and_survive_purge_and_attribute_changes() {
    let mut h = harness_with_queue();
    h.ok(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"a": "1"}}));
    let attrs = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(attrs["Attributes"].get("Tags"), None);
    h.ok(json!({"Command": "PurgeQueue", "QueueName": "q"}));
    h.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {}}));
    // CreateQueue on an existing queue with equal attributes succeeds and keeps its tags
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    assert_eq!(list(&mut h), json!({"a": "1"}));
}

#[test]
fn delete_queue_drops_tags() {
    let mut h = harness_with_queue();
    h.ok(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"a": "1"}}));
    h.ok(json!({"Command": "DeleteQueue", "QueueName": "q"}));
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    assert_eq!(list(&mut h), json!({}));
}
