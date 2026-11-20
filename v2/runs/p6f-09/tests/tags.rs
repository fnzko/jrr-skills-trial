mod common;

use common::Harness;
use serde_json::{Value, json};

fn queue() -> Harness {
    let mut h = Harness::new();
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    h
}

fn tag(h: &mut Harness, tags: Value) -> Value {
    h.ok(json!({"Command": "TagQueue", "QueueName": "q", "Tags": tags}))
}

fn tags(h: &mut Harness) -> Value {
    h.ok(json!({"Command": "ListQueueTags", "QueueName": "q"}))["Tags"].clone()
}

#[test]
fn new_queue_has_no_tags() {
    assert_eq!(tags(&mut queue()), json!({}));
}

#[test]
fn tagging_sets_and_replaces_values() {
    let mut h = queue();
    tag(&mut h, json!({"a": "1", "b": ""}));
    tag(&mut h, json!({"a": "2", "c": "3"}));
    assert_eq!(tags(&mut h), json!({"a": "2", "b": "", "c": "3"}));
    tag(&mut h, json!({}));
    assert_eq!(tags(&mut h), json!({"a": "2", "b": "", "c": "3"}));
}

#[test]
fn keys_are_case_sensitive() {
    let mut h = queue();
    tag(&mut h, json!({"a": "1", "A": "2"}));
    assert_eq!(tags(&mut h), json!({"a": "1", "A": "2"}));
}

#[test]
fn untagging_removes_given_keys_and_ignores_unknown_ones() {
    let mut h = queue();
    tag(&mut h, json!({"a": "1", "b": "2"}));
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["a", "zzz"]}));
    assert_eq!(tags(&mut h), json!({"b": "2"}));
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": []}));
    assert_eq!(tags(&mut h), json!({"b": "2"}));
}

#[test]
fn invalid_keys_and_values_are_invalid_parameter_values_and_change_nothing() {
    let mut h = queue();
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
        let code = h.err(json!({"Command": "TagQueue", "QueueName": "q", "Tags": bad}));
        assert_eq!(code, "InvalidParameterValue", "{bad}");
    }
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "q", "Tags": []})),
        "InvalidParameterValue"
    );
    for bad in [json!([1]), json!(["a b"]), json!([""]), json!({"a": 1})] {
        let code = h.err(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": bad}));
        assert_eq!(code, "InvalidParameterValue", "{bad}");
    }
    assert_eq!(tags(&mut h), json!({}));
}

#[test]
fn value_limits_are_inclusive_and_count_utf8_bytes() {
    let mut h = queue();
    let max_key = "k".repeat(128);
    tag(&mut h, json!({max_key: "v".repeat(256)}));
    // 128 two-byte characters are exactly 256 bytes.
    tag(&mut h, json!({"u": "\u{e9}".repeat(128)}));
    let code = h.err(json!({"Command": "TagQueue", "QueueName": "q",
        "Tags": {"u": "\u{e9}".repeat(128) + "a"}}));
    assert_eq!(code, "InvalidParameterValue");
}

#[test]
fn at_most_fifty_tags_and_replacing_does_not_count_as_new() {
    let mut h = queue();
    let fifty: serde_json::Map<String, Value> =
        (0..50).map(|i| (format!("k{i}"), json!("v"))).collect();
    tag(&mut h, Value::Object(fifty));
    tag(&mut h, json!({"k0": "new"}));
    let code = h
        .err(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"extra": "v", "k1": "new"}}));
    assert_eq!(code, "LimitExceeded");
    let listed = tags(&mut h);
    assert_eq!(listed.as_object().unwrap().len(), 50);
    assert_eq!(listed["k0"], "new");
    assert_eq!(listed["k1"], "v");
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["k2"]}));
    tag(&mut h, json!({"extra": "v"}));
}

#[test]
fn error_order_is_required_then_value_then_queue() {
    let mut h = queue();
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "q"})),
        "MissingParameter"
    );
    assert_eq!(
        h.err(json!({"Command": "UntagQueue", "TagKeys": []})),
        "MissingParameter"
    );
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "nope", "Tags": {"a b": "x"}})),
        "InvalidParameterValue"
    );
    for cmd in [
        json!({"Command": "TagQueue", "QueueName": "nope", "Tags": {}}),
        json!({"Command": "UntagQueue", "QueueName": "nope", "TagKeys": []}),
        json!({"Command": "ListQueueTags", "QueueName": "nope"}),
    ] {
        assert_eq!(h.err(cmd), "QueueDoesNotExist");
    }
}

#[test]
fn tags_are_not_attributes_and_survive_purge_but_not_delete() {
    let mut h = queue();
    tag(&mut h, json!({"a": "1"}));
    let attrs =
        h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}))["Attributes"].clone();
    assert!(attrs.get("Tags").is_none());
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    h.ok(json!({"Command": "PurgeQueue", "QueueName": "q"}));
    assert_eq!(tags(&mut h), json!({"a": "1"}));
    h.ok(json!({"Command": "DeleteQueue", "QueueName": "q"}));
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    assert_eq!(tags(&mut h), json!({}));
}
