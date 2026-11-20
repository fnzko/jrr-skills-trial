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
    h.ok(json!({"Command": "ListQueueTags", "QueueName": "q"}))["Tags"].clone()
}

#[test]
fn tag_set_replace_untag_list() {
    let mut h = setup();
    assert_eq!(list(&mut h), json!({}));
    assert_eq!(tag(&mut h, json!({"a": "1", "b": ""})), json!({"ok": true}));
    assert_eq!(tag(&mut h, json!({"a": "2"})), json!({"ok": true}));
    assert_eq!(list(&mut h), json!({"a": "2", "b": ""}));
    let r = h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["a", "zz"]}));
    assert_eq!(r, json!({"ok": true}));
    assert_eq!(list(&mut h), json!({"b": ""}));
}

#[test]
fn empty_tags_and_keys_are_accepted() {
    let mut h = setup();
    tag(&mut h, json!({"a": "1"}));
    assert_eq!(tag(&mut h, json!({})), json!({"ok": true}));
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": []}));
    assert_eq!(list(&mut h), json!({"a": "1"}));
}

#[test]
fn invalid_tag_values_and_keys() {
    let mut h = setup();
    let long_key = "k".repeat(129);
    let long_value = "v".repeat(257);
    let bad = [
        json!({"bad key": "x"}),
        json!({"": "x"}),
        json!({long_key: "x"}),
        json!({"a": 1}),
        json!({"a": null}),
        json!({"a": long_value}),
        json!(["a"]),
    ];
    for tags in bad {
        let r = tag(&mut h, tags.clone());
        assert_eq!(r["error"]["code"], "InvalidParameterValue", "{tags}");
    }
    assert_eq!(list(&mut h), json!({}));
    for keys in [json!([1]), json!(["a b"]), json!("a"), json!([""])] {
        let code = h.err(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": keys}));
        assert_eq!(code, "InvalidParameterValue", "{keys}");
    }
}

#[test]
fn value_limit_counts_utf8_bytes() {
    let mut h = setup();
    let ok_value = "\u{e9}".repeat(128);
    assert_eq!(tag(&mut h, json!({"a": ok_value}))["ok"], true);
    let too_long = "\u{e9}".repeat(129);
    assert_eq!(
        tag(&mut h, json!({"a": too_long}))["error"]["code"],
        "InvalidParameterValue"
    );
}

#[test]
fn fifty_tags_at_most_and_failure_changes_nothing() {
    let mut h = setup();
    let fifty: serde_json::Map<String, Value> =
        (0..50).map(|i| (format!("k{i}"), json!("v"))).collect();
    assert_eq!(tag(&mut h, Value::Object(fifty))["ok"], true);
    // Replacing an existing key is fine at the limit.
    assert_eq!(tag(&mut h, json!({"k0": "new"}))["ok"], true);
    let r = tag(&mut h, json!({"k0": "changed", "extra": "x"}));
    assert_eq!(r["error"]["code"], "LimitExceeded");
    let tags = list(&mut h);
    assert_eq!(tags.as_object().unwrap().len(), 50);
    assert_eq!(tags["k0"], "new");
    // Removing one makes room.
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["k1"]}));
    assert_eq!(tag(&mut h, json!({"extra": "x"}))["ok"], true);
}

#[test]
fn keys_are_case_sensitive() {
    let mut h = setup();
    tag(&mut h, json!({"a": "1", "A": "2"}));
    assert_eq!(list(&mut h), json!({"a": "1", "A": "2"}));
}

#[test]
fn error_order_missing_invalid_then_lookup() {
    let mut h = Harness::new();
    let missing = h.err(json!({"Command": "TagQueue", "QueueName": "nope"}));
    assert_eq!(missing, "MissingParameter");
    let invalid = h.err(json!({"Command": "TagQueue", "QueueName": "nope", "Tags": {"a b": "x"}}));
    assert_eq!(invalid, "InvalidParameterValue");
    for command in [
        json!({"Command": "TagQueue", "QueueName": "nope", "Tags": {}}),
        json!({"Command": "UntagQueue", "QueueName": "nope", "TagKeys": []}),
        json!({"Command": "ListQueueTags", "QueueName": "nope"}),
    ] {
        assert_eq!(h.err(command), "QueueDoesNotExist");
    }
    assert_eq!(
        h.err(json!({"Command": "UntagQueue", "QueueName": "nope"})),
        "MissingParameter"
    );
    assert_eq!(
        h.err(json!({"Command": "ListQueueTags"})),
        "MissingParameter"
    );
}

#[test]
fn tags_are_not_attributes_and_survive_purge_but_not_delete() {
    let mut h = setup();
    tag(&mut h, json!({"a": "1"}));
    let attrs = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert!(attrs["Attributes"].get("Tags").is_none());
    // CreateQueue rule 4 ignores tags.
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    h.ok(json!({"Command": "PurgeQueue", "QueueName": "q"}));
    assert_eq!(list(&mut h), json!({"a": "1"}));
    h.ok(json!({"Command": "DeleteQueue", "QueueName": "q"}));
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    assert_eq!(list(&mut h), json!({}));
}
