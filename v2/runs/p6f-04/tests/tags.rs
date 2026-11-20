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

fn tags(h: &mut Harness) -> Value {
    h.ok(json!({"Command": "ListQueueTags", "QueueName": "q"}))["Tags"].clone()
}

#[test]
fn new_queue_has_no_tags() {
    let mut h = harness();
    assert_eq!(tags(&mut h), json!({}));
}

#[test]
fn tag_sets_and_replaces_values() {
    let mut h = harness();
    tag(&mut h, json!({"a": "1", "b": ""}));
    tag(&mut h, json!({"a": "2", "C": "3"}));
    assert_eq!(tags(&mut h), json!({"a": "2", "b": "", "C": "3"}));
    tag(&mut h, json!({}));
    assert_eq!(tags(&mut h).as_object().unwrap().len(), 3);
}

#[test]
fn untag_removes_and_ignores_unknown_keys() {
    let mut h = harness();
    tag(&mut h, json!({"a": "1", "b": "2"}));
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["a", "zzz"]}));
    assert_eq!(tags(&mut h), json!({"b": "2"}));
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": []}));
    assert_eq!(tags(&mut h), json!({"b": "2"}));
}

#[test]
fn invalid_keys_and_values_are_invalid_parameter_values() {
    let mut h = harness();
    let long_key = "k".repeat(129);
    let long_value = "v".repeat(257);
    let two_byte_values = "\u{e9}".repeat(129);
    for bad in [
        json!({"": "x"}),
        json!({"a b": "x"}),
        json!({long_key: "x"}),
        json!({"a": 1}),
        json!({"a": null}),
        json!({"a": long_value}),
        json!({"a": two_byte_values}),
    ] {
        let code = h.err(json!({"Command": "TagQueue", "QueueName": "q", "Tags": bad}));
        assert_eq!(code, "InvalidParameterValue", "{bad}");
    }
    for bad in [json!([1]), json!(["a b"]), json!([""]), json!([null])] {
        let code = h.err(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": bad}));
        assert_eq!(code, "InvalidParameterValue", "{bad}");
    }
    assert_eq!(tags(&mut h), json!({}));
}

#[test]
fn boundary_lengths_are_accepted() {
    let mut h = harness();
    let key = "k".repeat(128);
    let value = "v".repeat(256);
    let two_byte_value = "\u{e9}".repeat(128);
    tag(
        &mut h,
        json!({key.clone(): value.clone(), "two": two_byte_value}),
    );
    assert_eq!(tags(&mut h)[&key], json!(value));
}

#[test]
fn wrong_types_and_missing_parameters() {
    let mut h = harness();
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
        h.err(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": {}})),
        "InvalidParameterValue"
    );
}

#[test]
fn value_checks_come_before_the_queue_lookup() {
    let mut h = harness();
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

#[test]
fn fifty_tags_fit_and_a_fifty_first_does_not() {
    let mut h = harness();
    let fifty: serde_json::Map<String, Value> =
        (0..50).map(|i| (format!("k{i}"), json!("v"))).collect();
    tag(&mut h, Value::Object(fifty));
    let over = json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"extra": "v"}});
    assert_eq!(h.err(over), "LimitExceeded");
    // Replacing existing keys does not add tags, and a failed command changes nothing.
    tag(&mut h, json!({"k0": "new"}));
    let mixed = json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"k1": "x", "extra": "v"}});
    assert_eq!(h.err(mixed), "LimitExceeded");
    let t = tags(&mut h);
    assert_eq!(t.as_object().unwrap().len(), 50);
    assert_eq!(t["k0"], "new");
    assert_eq!(t["k1"], "v");
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["k2"]}));
    tag(&mut h, json!({"extra": "v"}));
}

#[test]
fn tags_are_not_attributes_and_survive_purge_but_not_delete() {
    let mut h = harness();
    tag(&mut h, json!({"a": "1"}));
    let attrs = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert!(attrs["Attributes"].get("Tags").is_none());
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    h.ok(json!({"Command": "SendMessage", "QueueName": "q", "Body": "x"}));
    h.ok(json!({"Command": "PurgeQueue", "QueueName": "q"}));
    assert_eq!(tags(&mut h), json!({"a": "1"}));
    h.ok(json!({"Command": "DeleteQueue", "QueueName": "q"}));
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    assert_eq!(tags(&mut h), json!({}));
}

#[test]
fn tags_are_per_queue() {
    let mut h = harness();
    h.ok(json!({"Command": "CreateQueue", "QueueName": "other"}));
    tag(&mut h, json!({"a": "1"}));
    let r = h.ok(json!({"Command": "ListQueueTags", "QueueName": "other"}));
    assert_eq!(r["Tags"], json!({}));
}
