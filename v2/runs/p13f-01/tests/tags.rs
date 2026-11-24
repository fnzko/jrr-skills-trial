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
    h.ok(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"a": "1", "b": ""}}));
    h.ok(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"a": "2", "A": "3"}}));
    assert_eq!(list(&mut h), json!({"a": "2", "b": "", "A": "3"}));
}

#[test]
fn untag_removes_listed_keys_and_ignores_unknown_ones() {
    let mut h = harness_with_queue();
    tag(&mut h, json!({"a": "1", "b": "2"}));
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["a", "zzz"]}));
    assert_eq!(list(&mut h), json!({"b": "2"}));
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": []}));
    assert_eq!(list(&mut h), json!({"b": "2"}));
}

#[test]
fn invalid_keys_and_values_are_invalid_parameter_values() {
    let mut h = harness_with_queue();
    let long_key = "k".repeat(129);
    let long_value = "v".repeat(257);
    for tags in [
        json!({"": "x"}),
        json!({"bad key": "x"}),
        json!({"é": "x"}),
        json!({long_key: "x"}),
        json!({"k": 1}),
        json!({"k": null}),
        json!({"k": long_value}),
        json!({"ok": "x", "k": ["x"]}),
    ] {
        assert_eq!(
            tag(&mut h, tags.clone())["error"]["code"],
            "InvalidParameterValue",
            "{tags}"
        );
    }
    assert_eq!(list(&mut h), json!({}));
}

#[test]
fn limits_are_inclusive() {
    let mut h = harness_with_queue();
    let max_key = "k".repeat(128);
    let max_value = "é".repeat(128);
    assert_eq!(
        tag(&mut h, json!({max_key.clone(): max_value.clone()}))["ok"],
        true
    );
    assert_eq!(list(&mut h), json!({max_key: max_value}));
}

#[test]
fn untag_rejects_bad_key_elements() {
    let mut h = harness_with_queue();
    tag(&mut h, json!({"a": "1"}));
    for keys in [
        json!([1]),
        json!(["a", "bad key"]),
        json!([null]),
        json!("a"),
        json!({}),
    ] {
        assert_eq!(
            h.err(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": keys})),
            "InvalidParameterValue",
            "{keys}"
        );
    }
    assert_eq!(list(&mut h), json!({"a": "1"}));
}

fn numbered(range: std::ops::Range<u32>) -> Value {
    range
        .map(|i| (format!("k{i}"), json!("v")))
        .collect::<serde_json::Map<_, _>>()
        .into()
}

#[test]
fn fifty_tags_fit_and_fifty_one_do_not() {
    let mut h = harness_with_queue();
    assert_eq!(tag(&mut h, numbered(0..50))["ok"], true);
    assert_eq!(
        tag(&mut h, json!({"extra": "v"}))["error"]["code"],
        "LimitExceeded"
    );
    assert_eq!(tag(&mut h, json!({"k0": "new"}))["ok"], true);
    assert_eq!(list(&mut h).as_object().unwrap().len(), 50);
    assert_eq!(list(&mut h)["k0"], "new");
}

#[test]
fn failed_tag_queue_changes_nothing() {
    let mut h = harness_with_queue();
    tag(&mut h, numbered(0..49));
    assert_eq!(
        tag(&mut h, json!({"x": "1", "y": "2"}))["error"]["code"],
        "LimitExceeded"
    );
    assert_eq!(list(&mut h).as_object().unwrap().len(), 49);
    assert_eq!(list(&mut h).get("x"), None);
}

#[test]
fn check_order_for_tag_commands() {
    let mut h = harness_with_queue();
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
        h.err(json!({"Command": "TagQueue", "QueueName": "q", "Tags": []})),
        "InvalidParameterValue"
    );
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "nope", "Tags": {"bad key": "x"}})),
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
    assert_eq!(
        h.err(json!({"Command": "ListQueueTags"})),
        "MissingParameter"
    );
}

#[test]
fn tags_are_not_attributes_and_survive_purge_but_not_delete() {
    let mut h = harness_with_queue();
    tag(&mut h, json!({"a": "1"}));
    let attrs = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(attrs["Attributes"].get("Tags"), None);
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    h.ok(json!({"Command": "PurgeQueue", "QueueName": "q"}));
    assert_eq!(list(&mut h), json!({"a": "1"}));
    h.ok(json!({"Command": "DeleteQueue", "QueueName": "q"}));
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    assert_eq!(list(&mut h), json!({}));
}
