mod common;

use common::Harness;
use serde_json::{Value, json};

fn harness() -> Harness {
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
fn tags_are_empty_at_first_and_results_have_no_members() {
    let mut h = harness();
    assert_eq!(list(&mut h), json!({}));
    assert_eq!(
        h.run(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {}})),
        json!({"ok": true})
    );
    assert_eq!(
        h.run(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": []})),
        json!({"ok": true})
    );
}

#[test]
fn tagging_replaces_values_and_untagging_ignores_unknown_keys() {
    let mut h = harness();
    tag(&mut h, json!({"a": "1", "b": "", "A": "upper"}));
    tag(&mut h, json!({"a": "2"}));
    assert_eq!(list(&mut h), json!({"a": "2", "b": "", "A": "upper"}));
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["a", "zzz"]}));
    assert_eq!(list(&mut h), json!({"b": "", "A": "upper"}));
}

#[test]
fn invalid_tags_are_parameter_errors() {
    let mut h = harness();
    let tag_cmd = |tags: Value| json!({"Command": "TagQueue", "QueueName": "q", "Tags": tags});
    let long_key = "k".repeat(129);
    for bad in [
        json!({"": "v"}),
        json!({"a b": "v"}),
        json!({long_key: "v"}),
        json!({"k": 1}),
        json!({"k": null}),
        json!({"k": "x".repeat(257)}),
        json!([]),
        json!("a"),
    ] {
        assert_eq!(
            h.err(tag_cmd(bad.clone())),
            "InvalidParameterValue",
            "{bad}"
        );
    }
    for bad in [
        json!(["a", 1]),
        json!([""]),
        json!(["a!"]),
        json!({}),
        json!("a"),
    ] {
        assert_eq!(
            h.err(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": bad})),
            "InvalidParameterValue",
            "{bad}"
        );
    }
    assert_eq!(list(&mut h), json!({}));
    tag(&mut h, json!({"k".repeat(128): "é".repeat(128)}));
}

#[test]
fn missing_parameters_and_unknown_queue() {
    let mut h = harness();
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "q"})),
        "MissingParameter"
    );
    assert_eq!(
        h.err(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": null})),
        "MissingParameter"
    );
    assert_eq!(
        h.err(json!({"Command": "ListQueueTags"})),
        "MissingParameter"
    );
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "nope", "Tags": {}})),
        "QueueDoesNotExist"
    );
    assert_eq!(
        h.err(json!({"Command": "ListQueueTags", "QueueName": "nope"})),
        "QueueDoesNotExist"
    );
}

#[test]
fn more_than_fifty_tags_is_a_limit_and_changes_nothing() {
    let mut h = harness();
    let fifty: serde_json::Map<String, Value> =
        (0..50).map(|i| (format!("k{i}"), json!("v"))).collect();
    tag(&mut h, Value::Object(fifty));
    tag(&mut h, json!({"k0": "new"}));
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"k0": "x", "extra": "y"}})),
        "LimitExceeded"
    );
    let tags = list(&mut h);
    assert_eq!(tags.as_object().unwrap().len(), 50);
    assert_eq!(tags["k0"], "new");
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["k1"]}));
    tag(&mut h, json!({"extra": "y"}));
}

#[test]
fn tags_are_separate_from_attributes_purge_and_create() {
    let mut h = harness();
    tag(&mut h, json!({"a": "1"}));
    let attrs = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(attrs["Attributes"].get("Tags"), None);
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    h.ok(json!({"Command": "PurgeQueue", "QueueName": "q"}));
    assert_eq!(list(&mut h), json!({"a": "1"}));
}

#[test]
fn tags_disappear_with_the_queue() {
    let mut h = harness();
    tag(&mut h, json!({"a": "1"}));
    h.ok(json!({"Command": "DeleteQueue", "QueueName": "q"}));
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    assert_eq!(list(&mut h), json!({}));
}
