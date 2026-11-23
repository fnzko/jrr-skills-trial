mod common;

use common::Harness;
use serde_json::{Value, json};

fn queue() -> Harness {
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
    let mut h = queue();
    assert_eq!(list(&mut h), json!({}));
}

#[test]
fn tag_sets_and_replaces() {
    let mut h = queue();
    assert_eq!(tag(&mut h, json!({"a": "1", "b": ""})), json!({"ok": true}));
    assert_eq!(tag(&mut h, json!({"a": "2"})), json!({"ok": true}));
    assert_eq!(list(&mut h), json!({"a": "2", "b": ""}));
    assert_eq!(tag(&mut h, json!({})), json!({"ok": true}));
    assert_eq!(list(&mut h), json!({"a": "2", "b": ""}));
}

#[test]
fn keys_are_case_sensitive() {
    let mut h = queue();
    tag(&mut h, json!({"k": "1", "K": "2"}));
    assert_eq!(list(&mut h), json!({"k": "1", "K": "2"}));
}

#[test]
fn invalid_keys_and_values_are_rejected_without_effect() {
    let mut h = queue();
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
        let r = tag(&mut h, bad.clone());
        assert_eq!(r["error"]["code"], "InvalidParameterValue", "{bad}");
    }
    assert_eq!(list(&mut h), json!({}));
}

#[test]
fn value_limit_is_in_bytes_and_inclusive() {
    let mut h = queue();
    assert_eq!(tag(&mut h, json!({"a": "v".repeat(256)}))["ok"], true);
    // 129 two-byte characters are 258 bytes
    let r = tag(&mut h, json!({"b": "é".repeat(129)}));
    assert_eq!(r["error"]["code"], "InvalidParameterValue");
    assert_eq!(tag(&mut h, json!({"b": "é".repeat(128)}))["ok"], true);
}

#[test]
fn tag_parameter_errors() {
    let mut h = queue();
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "q"})),
        "MissingParameter"
    );
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "q", "Tags": []})),
        "InvalidParameterValue"
    );
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "nope", "Tags": {}})),
        "QueueDoesNotExist"
    );
    // parameter values are checked before the queue lookup
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "nope", "Tags": {"a": 1}})),
        "InvalidParameterValue"
    );
}

#[test]
fn fifty_tags_fit_and_the_fifty_first_does_not() {
    let mut h = queue();
    let fifty: serde_json::Map<String, Value> =
        (0..50).map(|i| (format!("k{i}"), json!("v"))).collect();
    assert_eq!(tag(&mut h, Value::Object(fifty))["ok"], true);
    assert_eq!(tag(&mut h, json!({"k0": "new", "k1": "new"}))["ok"], true);
    let r = tag(&mut h, json!({"extra": "v", "k0": "changed"}));
    assert_eq!(r["error"]["code"], "LimitExceeded");
    let tags = list(&mut h);
    assert_eq!(tags.as_object().unwrap().len(), 50);
    assert_eq!(tags["k0"], "new");
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["k0"]}));
    assert_eq!(tag(&mut h, json!({"extra": "v"}))["ok"], true);
}

#[test]
fn untag_removes_and_ignores_missing_keys() {
    let mut h = queue();
    tag(&mut h, json!({"a": "1", "b": "2"}));
    let r = h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["a", "zzz", "a"]}));
    assert_eq!(r, json!({"ok": true}));
    assert_eq!(list(&mut h), json!({"b": "2"}));
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": []}));
    assert_eq!(list(&mut h), json!({"b": "2"}));
}

#[test]
fn untag_parameter_errors() {
    let mut h = queue();
    tag(&mut h, json!({"a": "1"}));
    assert_eq!(
        h.err(json!({"Command": "UntagQueue", "QueueName": "q"})),
        "MissingParameter"
    );
    for bad in [
        json!({}),
        json!(["a", 1]),
        json!(["a", "bad key"]),
        json!([""]),
    ] {
        let c = json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": bad});
        assert_eq!(h.err(c), "InvalidParameterValue", "{bad}");
    }
    // a failed command removes nothing
    assert_eq!(list(&mut h), json!({"a": "1"}));
    assert_eq!(
        h.err(json!({"Command": "UntagQueue", "QueueName": "nope", "TagKeys": []})),
        "QueueDoesNotExist"
    );
}

#[test]
fn list_tags_of_missing_queue() {
    let mut h = Harness::new();
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
    let mut h = queue();
    tag(&mut h, json!({"a": "1"}));
    let r = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert!(r["Attributes"].get("Tags").is_none());
    // re-creating with the same attributes still succeeds
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    h.ok(json!({"Command": "PurgeQueue", "QueueName": "q"}));
    assert_eq!(list(&mut h), json!({"a": "1"}));
    h.ok(json!({"Command": "DeleteQueue", "QueueName": "q"}));
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    assert_eq!(list(&mut h), json!({}));
}
