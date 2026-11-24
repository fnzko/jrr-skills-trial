mod common;

use common::Harness;
use serde_json::{Value, json};

fn harness_with_queue() -> Harness {
    let mut h = Harness::new();
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    h
}

fn tag(h: &mut Harness, tags: Value) -> Value {
    h.ok(json!({"Command": "TagQueue", "QueueName": "q", "Tags": tags}))
}

fn list(h: &mut Harness) -> Value {
    h.ok(json!({"Command": "ListQueueTags", "QueueName": "q"}))["Tags"].clone()
}

#[test]
fn new_queue_has_no_tags_and_commands_return_no_members() {
    let mut h = harness_with_queue();
    assert_eq!(list(&mut h), json!({}));
    assert_eq!(tag(&mut h, json!({})), json!({"ok": true}));
    let untag = h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": []}));
    assert_eq!(untag, json!({"ok": true}));
}

#[test]
fn tagging_adds_and_replaces_and_untagging_ignores_unknown_keys() {
    let mut h = harness_with_queue();
    tag(&mut h, json!({"a": "1", "b": ""}));
    tag(&mut h, json!({"a": "2", "c": "3"}));
    assert_eq!(list(&mut h), json!({"a": "2", "b": "", "c": "3"}));
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["a", "zzz"]}));
    assert_eq!(list(&mut h), json!({"b": "", "c": "3"}));
}

#[test]
fn keys_are_case_sensitive() {
    let mut h = harness_with_queue();
    tag(&mut h, json!({"k": "1", "K": "2"}));
    assert_eq!(list(&mut h), json!({"k": "1", "K": "2"}));
}

#[test]
fn invalid_keys_and_values_are_invalid_parameter_values() {
    let mut h = harness_with_queue();
    let long_key = "k".repeat(129);
    let long_value = "é".repeat(129);
    for bad in [
        json!({"": "x"}),
        json!({"a b": "x"}),
        json!({long_key: "x"}),
        json!({"a": 1}),
        json!({"a": null}),
        json!({"a": long_value}),
    ] {
        let command = json!({"Command": "TagQueue", "QueueName": "q", "Tags": bad});
        assert_eq!(h.err(command), "InvalidParameterValue", "{bad}");
    }
    for bad in [json!(["a b"]), json!([1]), json!("a"), json!([null])] {
        let command = json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": bad});
        assert_eq!(h.err(command), "InvalidParameterValue", "{bad}");
    }
    assert_eq!(list(&mut h), json!({}));
}

#[test]
fn value_of_256_bytes_is_accepted() {
    let mut h = harness_with_queue();
    let value = "é".repeat(128);
    tag(&mut h, json!({"a": value}));
    assert_eq!(list(&mut h), json!({"a": "é".repeat(128)}));
}

#[test]
fn more_than_fifty_tags_is_limit_exceeded_and_changes_nothing() {
    let mut h = harness_with_queue();
    let fifty: serde_json::Map<String, Value> =
        (0..50).map(|i| (format!("k{i}"), json!("v"))).collect();
    tag(&mut h, Value::Object(fifty));
    let command = json!({"Command": "TagQueue", "QueueName": "q",
                         "Tags": {"k0": "new", "extra": "x"}});
    assert_eq!(h.err(command), "LimitExceeded");
    assert_eq!(list(&mut h)["k0"], "v");
    tag(&mut h, json!({"k0": "new"}));
    assert_eq!(list(&mut h)["k0"], "new");
}

#[test]
fn check_order_missing_then_value_then_queue() {
    let mut h = Harness::new();
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "nope"})),
        "MissingParameter"
    );
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "Tags": {}})),
        "MissingParameter"
    );
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "nope", "Tags": {"a b": "x"}})),
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
    let mut h = harness_with_queue();
    tag(&mut h, json!({"a": "1"}));
    let attributes = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(
        attributes["Attributes"],
        json!({
            "VisibilityTimeout": 30, "DelaySeconds": 0, "RetentionSeconds": 345600,
            "Fifo": false, "ContentBasedDeduplication": false, "MaxMessageBytes": 1024,
        })
    );
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    h.ok(json!({"Command": "PurgeQueue", "QueueName": "q"}));
    assert_eq!(list(&mut h), json!({"a": "1"}));
    h.ok(json!({"Command": "DeleteQueue", "QueueName": "q"}));
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    assert_eq!(list(&mut h), json!({}));
}
