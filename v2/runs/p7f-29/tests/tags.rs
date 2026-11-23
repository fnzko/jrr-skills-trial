mod common;

use common::Harness;
use serde_json::{Value, json};

fn setup() -> Harness {
    let mut h = Harness::new();
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    h
}

fn tag(h: &mut Harness, tags: Value) -> Value {
    h.run(json!({"Command": "TagQueue", "QueueName": "q", "Tags": tags}))
}

fn list(h: &mut Harness) -> Value {
    h.ok(json!({"Command": "ListQueueTags", "QueueName": "q"}))
}

#[test]
fn new_queue_has_no_tags() {
    let mut h = setup();
    assert_eq!(list(&mut h), json!({"ok": true, "Tags": {}}));
}

#[test]
fn tag_sets_and_replaces() {
    let mut h = setup();
    assert_eq!(tag(&mut h, json!({"a": "1", "B": ""})), json!({"ok": true}));
    assert_eq!(tag(&mut h, json!({"a": "2"})), json!({"ok": true}));
    assert_eq!(list(&mut h)["Tags"], json!({"a": "2", "B": ""}));
}

#[test]
fn empty_tags_object_is_a_success() {
    let mut h = setup();
    assert_eq!(tag(&mut h, json!({})), json!({"ok": true}));
}

#[test]
fn untag_removes_and_ignores_unknown_keys() {
    let mut h = setup();
    tag(&mut h, json!({"a": "1", "b": "2"}));
    let r = h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["a", "zzz"]}));
    assert_eq!(r, json!({"ok": true}));
    assert_eq!(list(&mut h)["Tags"], json!({"b": "2"}));
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": []}));
    assert_eq!(list(&mut h)["Tags"], json!({"b": "2"}));
}

#[test]
fn key_and_value_rules() {
    let mut h = setup();
    let long_key = "k".repeat(129);
    for bad in [
        json!({"": "v"}),
        json!({"a b": "v"}),
        json!({long_key: "v"}),
        json!({"a": 1}),
        json!({"a": null}),
        json!({"a": "x".repeat(257)}),
        // 129 two-byte characters.
        json!({"a": "\u{e9}".repeat(129)}),
    ] {
        assert_eq!(
            h.err(json!({"Command": "TagQueue", "QueueName": "q", "Tags": bad})),
            "InvalidParameterValue"
        );
    }
    assert_eq!(
        tag(&mut h, json!({"k".repeat(128): "x".repeat(256)}))["ok"],
        true
    );
    assert_eq!(tag(&mut h, json!({"b": "\u{e9}".repeat(128)}))["ok"], true);
    assert_eq!(list(&mut h)["Tags"].as_object().unwrap().len(), 2);
}

#[test]
fn a_bad_tag_changes_nothing() {
    let mut h = setup();
    assert_eq!(
        tag(&mut h, json!({"good": "v", "bad key": "v"}))["ok"],
        false
    );
    assert_eq!(list(&mut h)["Tags"], json!({}));
}

#[test]
fn untag_rejects_bad_keys_without_removing() {
    let mut h = setup();
    tag(&mut h, json!({"a": "1"}));
    for bad in [
        json!([1]),
        json!(["a", "b c"]),
        json!([""]),
        json!("a"),
        json!({}),
    ] {
        assert_eq!(
            h.err(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": bad})),
            "InvalidParameterValue"
        );
    }
    assert_eq!(list(&mut h)["Tags"], json!({"a": "1"}));
}

#[test]
fn at_most_fifty_tags() {
    let mut h = setup();
    let fifty: serde_json::Map<String, Value> =
        (0..50).map(|i| (format!("k{i}"), json!("v"))).collect();
    assert_eq!(tag(&mut h, Value::Object(fifty))["ok"], true);
    // Replacing an existing key does not add a tag.
    assert_eq!(tag(&mut h, json!({"k0": "new"}))["ok"], true);
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"extra": "v"}})),
        "LimitExceeded"
    );
    // Nothing of a failed command is applied, even the replacing part.
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"k1": "changed", "extra": "v"}})),
        "LimitExceeded"
    );
    let tags = list(&mut h)["Tags"].clone();
    assert_eq!(tags.as_object().unwrap().len(), 50);
    assert_eq!(tags["k1"], "v");
    assert_eq!(tags["k0"], "new");
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["k0"]}));
    assert_eq!(tag(&mut h, json!({"extra": "v"}))["ok"], true);
}

#[test]
fn error_check_order() {
    let mut h = setup();
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
    assert_eq!(
        h.err(json!({"Command": "ListQueueTags", "QueueName": 1})),
        "InvalidParameterValue"
    );
    // Value errors come before the queue lookup.
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "none", "Tags": {"a": 1}})),
        "InvalidParameterValue"
    );
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "none", "Tags": {}})),
        "QueueDoesNotExist"
    );
    assert_eq!(
        h.err(json!({"Command": "UntagQueue", "QueueName": "none", "TagKeys": []})),
        "QueueDoesNotExist"
    );
    assert_eq!(
        h.err(json!({"Command": "ListQueueTags", "QueueName": "none"})),
        "QueueDoesNotExist"
    );
}

#[test]
fn tags_are_not_attributes_and_survive_purge_but_not_delete() {
    let mut h = setup();
    tag(&mut h, json!({"a": "1"}));
    let attrs = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(
        attrs["Attributes"],
        json!({
            "VisibilityTimeout": 30, "DelaySeconds": 0, "RetentionSeconds": 345600,
            "Fifo": false, "ContentBasedDeduplication": false, "MaxMessageBytes": 1024,
        })
    );
    // CreateQueue rule 4 ignores tags.
    assert_eq!(
        h.run(json!({"Command": "CreateQueue", "QueueName": "q"})),
        json!({"ok": true})
    );
    h.ok(json!({"Command": "SendMessage", "QueueName": "q", "Body": "x"}));
    h.ok(json!({"Command": "PurgeQueue", "QueueName": "q"}));
    assert_eq!(list(&mut h)["Tags"], json!({"a": "1"}));
    h.ok(json!({"Command": "DeleteQueue", "QueueName": "q"}));
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    assert_eq!(list(&mut h)["Tags"], json!({}));
}
