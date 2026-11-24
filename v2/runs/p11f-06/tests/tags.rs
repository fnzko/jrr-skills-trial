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
    h.ok(json!({"Command": "ListQueueTags", "QueueName": "q"}))
}

#[test]
fn new_queue_has_no_tags() {
    let mut h = harness_with_queue();
    assert_eq!(list(&mut h), json!({"ok": true, "Tags": {}}));
}

#[test]
fn tag_sets_and_replaces() {
    let mut h = harness_with_queue();
    assert_eq!(tag(&mut h, json!({"a": "1", "B": ""})), json!({"ok": true}));
    assert_eq!(tag(&mut h, json!({"a": "2"})), json!({"ok": true}));
    assert_eq!(list(&mut h)["Tags"], json!({"a": "2", "B": ""}));
}

#[test]
fn empty_tags_object_is_accepted() {
    let mut h = harness_with_queue();
    assert_eq!(tag(&mut h, json!({})), json!({"ok": true}));
}

#[test]
fn invalid_tags_are_parameter_errors_before_lookup() {
    let mut h = Harness::new();
    let long_value = "v".repeat(257);
    let cases = [
        json!({"": "x"}),
        json!({"a b": "x"}),
        json!({"k".repeat(129): "x"}),
        json!({"a": 1}),
        json!({"a": null}),
        json!({"a": long_value}),
        json!("not an object"),
    ];
    for tags in cases {
        let r = tag(&mut h, tags.clone());
        assert_eq!(r["error"]["code"], "InvalidParameterValue", "{tags}");
    }
}

#[test]
fn value_limit_counts_utf8_bytes() {
    let mut h = harness_with_queue();
    let ok = "é".repeat(128);
    assert_eq!(tag(&mut h, json!({"a": ok})), json!({"ok": true}));
    let too_long = format!("{}x", "é".repeat(128));
    assert_eq!(
        tag(&mut h, json!({"a": too_long}))["error"]["code"],
        "InvalidParameterValue"
    );
}

#[test]
fn missing_and_unknown_queue() {
    let mut h = Harness::new();
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "q"})),
        "MissingParameter"
    );
    assert_eq!(
        h.err(json!({"Command": "UntagQueue", "QueueName": "q"})),
        "MissingParameter"
    );
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {}})),
        "QueueDoesNotExist"
    );
    assert_eq!(
        h.err(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": []})),
        "QueueDoesNotExist"
    );
    assert_eq!(
        h.err(json!({"Command": "ListQueueTags", "QueueName": "q"})),
        "QueueDoesNotExist"
    );
}

#[test]
fn fifty_tags_fit_and_the_fifty_first_does_not() {
    let mut h = harness_with_queue();
    let fifty: serde_json::Map<String, Value> =
        (0..50).map(|i| (format!("k{i}"), json!("v"))).collect();
    assert_eq!(tag(&mut h, Value::Object(fifty)), json!({"ok": true}));
    // Replacing an existing key stays within the limit.
    assert_eq!(tag(&mut h, json!({"k0": "new"})), json!({"ok": true}));
    // A failed command changes nothing, even for the keys that would fit.
    let r = tag(&mut h, json!({"k1": "changed", "extra": "x"}));
    assert_eq!(r["error"]["code"], "LimitExceeded");
    let tags = list(&mut h)["Tags"].clone();
    assert_eq!(tags["k1"], "v");
    assert_eq!(tags["k0"], "new");
    assert_eq!(tags.get("extra"), None);
    assert_eq!(tags.as_object().map(|t| t.len()), Some(50));
}

#[test]
fn too_many_new_tags_in_one_command_exceed_the_limit() {
    let mut h = harness_with_queue();
    let fifty_one: serde_json::Map<String, Value> =
        (0..51).map(|i| (format!("k{i}"), json!("v"))).collect();
    assert_eq!(
        tag(&mut h, Value::Object(fifty_one))["error"]["code"],
        "LimitExceeded"
    );
    assert_eq!(list(&mut h)["Tags"], json!({}));
}

#[test]
fn untag_removes_and_ignores_missing_keys() {
    let mut h = harness_with_queue();
    tag(&mut h, json!({"a": "1", "b": "2"}));
    let r = h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["a", "zzz", "a"]}));
    assert_eq!(r, json!({"ok": true}));
    assert_eq!(list(&mut h)["Tags"], json!({"b": "2"}));
}

#[test]
fn untag_validates_every_key() {
    let mut h = harness_with_queue();
    tag(&mut h, json!({"a": "1"}));
    for keys in [
        json!(["a", 1]),
        json!(["a", "b c"]),
        json!(["a", ""]),
        json!("a"),
        json!({}),
    ] {
        let code = h.err(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": keys}));
        assert_eq!(code, "InvalidParameterValue", "{keys}");
    }
    assert_eq!(list(&mut h)["Tags"], json!({"a": "1"}));
}

#[test]
fn tags_are_not_attributes_and_survive_purge_but_not_delete() {
    let mut h = harness_with_queue();
    tag(&mut h, json!({"a": "1"}));
    let attrs = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(attrs["Attributes"].get("Tags"), None);
    // CreateQueue rule 4 compares attributes only.
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    h.ok(json!({"Command": "PurgeQueue", "QueueName": "q"}));
    assert_eq!(list(&mut h)["Tags"], json!({"a": "1"}));
    h.ok(json!({"Command": "DeleteQueue", "QueueName": "q"}));
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    assert_eq!(list(&mut h)["Tags"], json!({}));
}

#[test]
fn tags_are_per_queue() {
    let mut h = harness_with_queue();
    h.ok(json!({"Command": "CreateQueue", "QueueName": "other"}));
    tag(&mut h, json!({"a": "1"}));
    let r = h.ok(json!({"Command": "ListQueueTags", "QueueName": "other"}));
    assert_eq!(r["Tags"], json!({}));
}
