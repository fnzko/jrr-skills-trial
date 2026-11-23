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

fn many(count: usize) -> Value {
    Value::Object((0..count).map(|i| (format!("k{i}"), json!("v"))).collect())
}

#[test]
fn a_new_queue_has_no_tags() {
    let mut h = harness();
    assert_eq!(tags(&mut h), json!({}));
}

#[test]
fn tag_sets_and_replaces_values() {
    let mut h = harness();
    assert_eq!(tag(&mut h, json!({"a": "1", "b": ""}))["ok"], true);
    assert_eq!(tag(&mut h, json!({"a": "2", "c": "3"}))["ok"], true);
    assert_eq!(tags(&mut h), json!({"a": "2", "b": "", "c": "3"}));
}

#[test]
fn keys_are_case_sensitive() {
    let mut h = harness();
    tag(&mut h, json!({"a": "1", "A": "2"}));
    assert_eq!(tags(&mut h), json!({"a": "1", "A": "2"}));
}

#[test]
fn invalid_keys_and_values_are_rejected_without_effect() {
    let mut h = harness();
    let long_key = "k".repeat(129);
    let long_value = "v".repeat(257);
    for bad in [
        json!({"ok": "1", "": "x"}),
        json!({"ok": "1", "bad key": "x"}),
        json!({"ok": "1", long_key: "x"}),
        json!({"ok": "1", "k": 5}),
        json!({"ok": "1", "k": null}),
        json!({"ok": "1", "k": long_value}),
    ] {
        let response = tag(&mut h, bad.clone());
        assert_eq!(response["error"]["code"], "InvalidParameterValue", "{bad}");
    }
    assert_eq!(tags(&mut h), json!({}));
}

#[test]
fn key_and_value_limits_are_inclusive_and_count_bytes() {
    let mut h = harness();
    let key = "k".repeat(128);
    // 128 two-byte characters are 256 bytes.
    let value = "é".repeat(128);
    assert_eq!(tag(&mut h, json!({key.clone(): value.clone()}))["ok"], true);
    assert_eq!(tags(&mut h), json!({key: value}));
    let too_long = "é".repeat(128) + "a";
    assert_eq!(
        tag(&mut h, json!({"x": too_long}))["error"]["code"],
        "InvalidParameterValue"
    );
}

#[test]
fn fifty_tags_fit_and_a_fifty_first_does_not() {
    let mut h = harness();
    assert_eq!(tag(&mut h, many(50))["ok"], true);
    assert_eq!(
        tag(&mut h, json!({"extra": "x"}))["error"]["code"],
        "LimitExceeded"
    );
    // Replacing an existing key does not add a tag.
    assert_eq!(tag(&mut h, json!({"k0": "new"}))["ok"], true);
    assert_eq!(tags(&mut h).as_object().unwrap().len(), 50);
    assert_eq!(tags(&mut h)["k0"], "new");
}

#[test]
fn over_the_limit_changes_nothing() {
    let mut h = harness();
    tag(&mut h, many(49));
    let response = tag(&mut h, json!({"k0": "new", "n1": "x", "n2": "y"}));
    assert_eq!(response["error"]["code"], "LimitExceeded");
    assert_eq!(tags(&mut h)["k0"], "v");
    assert_eq!(tags(&mut h).as_object().unwrap().len(), 49);
}

#[test]
fn empty_tags_object_is_accepted() {
    let mut h = harness();
    assert_eq!(tag(&mut h, json!({}))["ok"], true);
}

#[test]
fn untag_removes_given_keys_and_ignores_unknown_ones() {
    let mut h = harness();
    tag(&mut h, json!({"a": "1", "b": "2", "c": "3"}));
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["a", "zzz", "c"]}));
    assert_eq!(tags(&mut h), json!({"b": "2"}));
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": []}));
    assert_eq!(tags(&mut h), json!({"b": "2"}));
}

#[test]
fn untag_rejects_bad_elements_without_effect() {
    let mut h = harness();
    tag(&mut h, json!({"a": "1"}));
    for bad in [
        json!(["a", 5]),
        json!(["a", "bad key"]),
        json!(["a", ""]),
        json!(["a", null]),
        json!("a"),
    ] {
        assert_eq!(
            h.err(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": bad})),
            "InvalidParameterValue"
        );
    }
    assert_eq!(tags(&mut h), json!({"a": "1"}));
}

#[test]
fn check_order_for_tag_commands() {
    let mut h = harness();
    for cmd in ["TagQueue", "UntagQueue", "ListQueueTags"] {
        assert_eq!(h.err(json!({"Command": cmd})), "MissingParameter");
        assert_eq!(
            h.err(json!({"Command": cmd, "QueueName": "nope", "Tags": {}, "TagKeys": []})),
            "QueueDoesNotExist"
        );
    }
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": 1})),
        "MissingParameter"
    );
    assert_eq!(
        h.err(json!({"Command": "ListQueueTags", "QueueName": 1})),
        "InvalidParameterValue"
    );
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": 1})),
        "MissingParameter"
    );
    assert_eq!(
        h.err(json!({"Command": "ListQueueTags", "QueueName": 1})),
        "InvalidParameterValue"
    );
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "nope", "Tags": {"bad key": "x"}})),
        "InvalidParameterValue"
    );
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "q", "Tags": []})),
        "InvalidParameterValue"
    );
    assert_eq!(
        h.err(json!({"Command": "UntagQueue", "QueueName": "nope", "TagKeys": [1]})),
        "InvalidParameterValue"
    );
}

#[test]
fn tags_are_not_attributes_and_survive_purge_but_not_delete() {
    let mut h = harness();
    tag(&mut h, json!({"a": "1"}));
    let attributes = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(
        attributes["Attributes"],
        json!({
            "VisibilityTimeout": 30, "DelaySeconds": 0, "RetentionSeconds": 345600,
            "Fifo": false, "ContentBasedDeduplication": false, "MaxMessageBytes": 1024,
        })
    );
    // CreateQueue rule 4 ignores tags.
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    h.ok(json!({"Command": "PurgeQueue", "QueueName": "q"}));
    assert_eq!(tags(&mut h), json!({"a": "1"}));
    h.ok(json!({"Command": "DeleteQueue", "QueueName": "q"}));
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    assert_eq!(tags(&mut h), json!({}));
}
