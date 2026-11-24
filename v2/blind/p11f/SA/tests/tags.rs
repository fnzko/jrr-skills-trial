mod common;

use common::Harness;
use serde_json::{Value, json};

fn queue() -> Harness {
    let mut h = Harness::new();
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    h
}

fn tag(h: &mut Harness, tags: Value) {
    h.ok(json!({"Command": "TagQueue", "QueueName": "q", "Tags": tags}));
}

fn list(h: &mut Harness) -> Value {
    h.ok(json!({"Command": "ListQueueTags", "QueueName": "q"}))["Tags"].clone()
}

#[test]
fn a_new_queue_has_no_tags() {
    let mut h = queue();
    assert_eq!(list(&mut h), json!({}));
}

#[test]
fn tagging_sets_and_replaces_values() {
    let mut h = queue();
    tag(&mut h, json!({"env": "dev", "team": "a"}));
    tag(&mut h, json!({"env": "prod", "empty": ""}));
    assert_eq!(
        list(&mut h),
        json!({"env": "prod", "team": "a", "empty": ""})
    );
    tag(&mut h, json!({}));
    assert_eq!(list(&mut h).as_object().unwrap().len(), 3);
}

#[test]
fn keys_are_case_sensitive() {
    let mut h = queue();
    tag(&mut h, json!({"k": "1", "K": "2"}));
    assert_eq!(list(&mut h), json!({"k": "1", "K": "2"}));
}

#[test]
fn untagging_removes_given_keys_and_ignores_unknown_ones() {
    let mut h = queue();
    tag(&mut h, json!({"a": "1", "b": "2"}));
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["a", "zzz"]}));
    assert_eq!(list(&mut h), json!({"b": "2"}));
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": []}));
    assert_eq!(list(&mut h), json!({"b": "2"}));
}

#[test]
fn invalid_keys_and_values_change_nothing() {
    let mut h = queue();
    let long_key = "k".repeat(129);
    let long_value = "v".repeat(257);
    for bad in [
        json!({"ok": "1", "": "x"}),
        json!({"ok": "1", "a b": "x"}),
        json!({"ok": "1", long_key: "x"}),
        json!({"ok": "1", "n": 5}),
        json!({"ok": "1", "n": null}),
        json!({"ok": "1", "n": long_value}),
        json!({"ok": "1", "n": "é".repeat(129)}),
    ] {
        let code = h.err(json!({"Command": "TagQueue", "QueueName": "q", "Tags": bad}));
        assert_eq!(code, "InvalidParameterValue", "{bad}");
    }
    assert_eq!(list(&mut h), json!({}));
    tag(&mut h, json!({"k".repeat(128): "v".repeat(256)}));
}

#[test]
fn untag_rejects_bad_elements_and_removes_nothing() {
    let mut h = queue();
    tag(&mut h, json!({"a": "1"}));
    for bad in [
        json!(["a", 1]),
        json!(["a", null]),
        json!(["a", "b c"]),
        json!(["a", ""]),
    ] {
        let code = h.err(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": bad}));
        assert_eq!(code, "InvalidParameterValue", "{bad}");
    }
    assert_eq!(list(&mut h), json!({"a": "1"}));
}

#[test]
fn parameter_types_and_requirements() {
    let mut h = queue();
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "q"})),
        "MissingParameter"
    );
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "q", "Tags": null})),
        "MissingParameter"
    );
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "q", "Tags": []})),
        "InvalidParameterValue"
    );
    assert_eq!(
        h.err(json!({"Command": "UntagQueue", "QueueName": "q"})),
        "MissingParameter"
    );
    assert_eq!(
        h.err(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": {}})),
        "InvalidParameterValue"
    );
    assert_eq!(
        h.err(json!({"Command": "ListQueueTags"})),
        "MissingParameter"
    );
}

#[test]
fn value_errors_come_before_the_queue_lookup() {
    let mut h = Harness::new();
    let bad = json!({"Command": "TagQueue", "QueueName": "nope", "Tags": {"a b": "x"}});
    assert_eq!(h.err(bad), "InvalidParameterValue");
    let good = json!({"Command": "TagQueue", "QueueName": "nope", "Tags": {"a": "x"}});
    assert_eq!(h.err(good), "QueueDoesNotExist");
    assert_eq!(
        h.err(json!({"Command": "UntagQueue", "QueueName": "nope", "TagKeys": []})),
        "QueueDoesNotExist"
    );
    assert_eq!(
        h.err(json!({"Command": "ListQueueTags", "QueueName": "nope"})),
        "QueueDoesNotExist"
    );
}

fn numbered(range: std::ops::Range<usize>) -> Value {
    Value::Object(range.map(|i| (format!("k{i}"), json!("v"))).collect())
}

#[test]
fn at_most_fifty_tags() {
    let mut h = queue();
    tag(&mut h, numbered(0..50));
    // Replacing existing keys stays within the limit.
    tag(&mut h, numbered(0..50));
    let code = h.err(json!({"Command": "TagQueue", "QueueName": "q", "Tags": numbered(49..51)}));
    assert_eq!(code, "LimitExceeded");
    assert_eq!(list(&mut h).as_object().unwrap().len(), 50);
    assert_eq!(list(&mut h)["k49"], "v");
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["k0"]}));
    tag(&mut h, json!({"new": "x"}));
}

#[test]
fn more_than_fifty_given_at_once_is_a_limit_error() {
    let mut h = queue();
    let code = h.err(json!({"Command": "TagQueue", "QueueName": "q", "Tags": numbered(0..51)}));
    assert_eq!(code, "LimitExceeded");
    assert_eq!(list(&mut h), json!({}));
}

#[test]
fn tags_are_not_attributes_and_survive_purge_and_idempotent_create() {
    let mut h = queue();
    tag(&mut h, json!({"a": "1"}));
    let attrs = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert!(attrs["Attributes"].get("Tags").is_none());
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    h.ok(json!({"Command": "PurgeQueue", "QueueName": "q"}));
    assert_eq!(list(&mut h), json!({"a": "1"}));
}

#[test]
fn deleting_the_queue_drops_its_tags() {
    let mut h = queue();
    tag(&mut h, json!({"a": "1"}));
    h.ok(json!({"Command": "DeleteQueue", "QueueName": "q"}));
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    assert_eq!(list(&mut h), json!({}));
}
