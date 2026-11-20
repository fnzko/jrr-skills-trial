mod common;

use common::Harness;
use serde_json::{Value, json};

fn tag(h: &mut Harness, tags: Value) -> Value {
    h.run(json!({"Command": "TagQueue", "QueueName": "q", "Tags": tags}))
}

fn tags(h: &mut Harness) -> Value {
    h.ok(json!({"Command": "ListQueueTags", "QueueName": "q"}))["Tags"].clone()
}

fn harness_with_queue() -> Harness {
    let mut h = Harness::new();
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    h
}

#[test]
fn new_queue_has_no_tags() {
    let mut h = harness_with_queue();
    assert_eq!(tags(&mut h), json!({}));
}

#[test]
fn tag_sets_and_replaces() {
    let mut h = harness_with_queue();
    assert_eq!(tag(&mut h, json!({"a": "1", "B": ""})), json!({"ok": true}));
    assert_eq!(tags(&mut h), json!({"a": "1", "B": ""}));
    tag(&mut h, json!({"a": "2", "c": "3"}));
    assert_eq!(tags(&mut h), json!({"a": "2", "B": "", "c": "3"}));
    // An empty object is allowed and changes nothing.
    assert_eq!(tag(&mut h, json!({}))["ok"], true);
    assert_eq!(tags(&mut h), json!({"a": "2", "B": "", "c": "3"}));
}

#[test]
fn untag_removes_and_ignores_unknown_keys() {
    let mut h = harness_with_queue();
    tag(&mut h, json!({"a": "1", "b": "2"}));
    let r = h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["a", "zzz"]}));
    assert_eq!(r, json!({"ok": true}));
    assert_eq!(tags(&mut h), json!({"b": "2"}));
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": []}));
    assert_eq!(tags(&mut h), json!({"b": "2"}));
}

#[test]
fn invalid_keys_and_values_change_nothing() {
    let mut h = harness_with_queue();
    let long_key = "k".repeat(129);
    let long_value = "v".repeat(257);
    for bad in [
        json!({"ok": "1", "bad key": "1"}),
        json!({"": "1"}),
        json!({long_key: "1"}),
        json!({"ok": 1}),
        json!({"ok": null}),
        json!({"ok": long_value}),
    ] {
        assert_eq!(tag(&mut h, bad.clone())["error"]["code"], "InvalidParameterValue", "{bad}");
    }
    assert_eq!(tags(&mut h), json!({}));
    for bad in [json!(["a", 1]), json!(["a b"]), json!([""]), json!("a"), json!({})] {
        assert_eq!(
            h.err(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": bad})),
            "InvalidParameterValue",
            "{bad}"
        );
    }
}

#[test]
fn key_and_value_size_limits_are_inclusive() {
    let mut h = harness_with_queue();
    let key = "k".repeat(128);
    // 128 two-byte characters: 256 bytes.
    let value = "é".repeat(128);
    assert_eq!(tag(&mut h, json!({key.clone(): value.clone()}))["ok"], true);
    assert_eq!(tags(&mut h), json!({key: value}));
    // 129 two-byte characters: 258 bytes, though only 129 characters.
    assert_eq!(tag(&mut h, json!({"x": "é".repeat(129)}))["error"]["code"], "InvalidParameterValue");
}

#[test]
fn parameter_checks_precede_queue_lookup() {
    let mut h = Harness::new();
    assert_eq!(h.err(json!({"Command": "TagQueue", "QueueName": "q"})), "MissingParameter");
    assert_eq!(h.err(json!({"Command": "TagQueue", "Tags": {}})), "MissingParameter");
    assert_eq!(h.err(json!({"Command": "UntagQueue", "QueueName": "q"})), "MissingParameter");
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"a b": "1"}})),
        "InvalidParameterValue"
    );
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {}})),
        "QueueDoesNotExist"
    );
    assert_eq!(h.err(json!({"Command": "ListQueueTags", "QueueName": "q"})), "QueueDoesNotExist");
    assert_eq!(h.err(json!({"Command": "ListQueueTags"})), "MissingParameter");
}

#[test]
fn fifty_tags_fit_and_a_fifty_first_does_not() {
    let mut h = harness_with_queue();
    let fifty: serde_json::Map<String, Value> =
        (0..50).map(|i| (format!("k{i}"), json!("v"))).collect();
    assert_eq!(tag(&mut h, Value::Object(fifty.clone()))["ok"], true);
    // Replacing existing keys stays within the limit.
    assert_eq!(tag(&mut h, json!({"k0": "new"}))["ok"], true);
    let r = tag(&mut h, json!({"k0": "newer", "extra": "x"}));
    assert_eq!(r["error"]["code"], "LimitExceeded");
    // The failed command changed nothing, k0 included.
    assert_eq!(tags(&mut h)["k0"], "new");
    assert_eq!(tags(&mut h).as_object().unwrap().len(), 50);
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["k1"]}));
    assert_eq!(tag(&mut h, json!({"extra": "x"}))["ok"], true);
}

#[test]
fn tags_are_separate_from_attributes_purge_and_create() {
    let mut h = harness_with_queue();
    tag(&mut h, json!({"a": "1"}));
    let attrs = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert!(attrs["Attributes"].get("Tags").is_none());
    assert_eq!(attrs["Attributes"].as_object().unwrap().len(), 6);
    // Same attributes, so CreateQueue succeeds although the queue has tags.
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    h.ok(json!({"Command": "PurgeQueue", "QueueName": "q"}));
    assert_eq!(tags(&mut h), json!({"a": "1"}));
    h.ok(json!({"Command": "DeleteQueue", "QueueName": "q"}));
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    assert_eq!(tags(&mut h), json!({}));
}
