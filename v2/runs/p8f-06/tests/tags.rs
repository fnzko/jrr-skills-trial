mod common;

use common::Harness;
use serde_json::{Value, json};

fn new_queue() -> Harness {
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
fn queue_starts_without_tags() {
    let mut h = new_queue();
    assert_eq!(tags(&mut h), json!({}));
}

#[test]
fn tag_sets_and_replaces() {
    let mut h = new_queue();
    tag(&mut h, json!({"env": "prod", "Team": ""}));
    tag(&mut h, json!({"env": "dev"}));
    tag(&mut h, json!({}));
    assert_eq!(tags(&mut h), json!({"env": "dev", "Team": ""}));
}

#[test]
fn untag_removes_and_ignores_unknown_keys() {
    let mut h = new_queue();
    tag(&mut h, json!({"a": "1", "b": "2"}));
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["a", "zzz", "a"]}));
    assert_eq!(tags(&mut h), json!({"b": "2"}));
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": []}));
    assert_eq!(tags(&mut h), json!({"b": "2"}));
}

#[test]
fn invalid_keys_and_values() {
    let mut h = new_queue();
    let long_key = "k".repeat(129);
    let long_value = "v".repeat(257);
    for bad in [
        json!({"": "x"}),
        json!({"a b": "x"}),
        json!({long_key.clone(): "x"}),
        json!({"a": 1}),
        json!({"a": null}),
        json!({"a": long_value}),
        json!({"a": "\u{e9}".repeat(129)}),
    ] {
        let code = h.err(json!({"Command": "TagQueue", "QueueName": "q", "Tags": bad}));
        assert_eq!(code, "InvalidParameterValue", "{bad}");
    }
    assert_eq!(tags(&mut h), json!({}));
    tag(&mut h, json!({"k".repeat(128): "v".repeat(256), "b": "\u{e9}".repeat(128)}));
    assert_eq!(tags(&mut h).as_object().unwrap().len(), 2);
    for bad in [json!([1]), json!(["a b"]), json!([long_key]), json!("a"), json!([null])] {
        let code = h.err(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": bad}));
        assert_eq!(code, "InvalidParameterValue", "{bad}");
    }
}

#[test]
fn parameter_errors_precede_queue_lookup() {
    let mut h = Harness::new();
    assert_eq!(h.err(json!({"Command": "TagQueue", "QueueName": "q"})), "MissingParameter");
    assert_eq!(h.err(json!({"Command": "UntagQueue", "QueueName": "q"})), "MissingParameter");
    assert_eq!(h.err(json!({"Command": "ListQueueTags"})), "MissingParameter");
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "q", "Tags": []})),
        "InvalidParameterValue"
    );
    assert_eq!(
        h.err(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": {}})),
        "InvalidParameterValue"
    );
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {}})),
        "QueueDoesNotExist"
    );
    assert_eq!(
        h.err(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": []})),
        "QueueDoesNotExist"
    );
    assert_eq!(h.err(json!({"Command": "ListQueueTags", "QueueName": "q"})), "QueueDoesNotExist");
}

fn many(range: std::ops::Range<u32>) -> Value {
    Value::Object(range.map(|i| (format!("k{i}"), json!("v"))).collect())
}

#[test]
fn at_most_50_tags() {
    let mut h = new_queue();
    tag(&mut h, many(0..50));
    // Replacing an existing key does not add a tag.
    tag(&mut h, json!({"k0": "new"}));
    let code = h.err(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"extra": "x"}}));
    assert_eq!(code, "LimitExceeded");
    assert_eq!(tags(&mut h).as_object().unwrap().len(), 50);
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["k1"]}));
    tag(&mut h, json!({"extra": "x"}));
}

#[test]
fn over_limit_batch_changes_nothing() {
    let mut h = new_queue();
    tag(&mut h, many(0..49));
    let code = h.err(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"a": "1", "b": "2", "k0": "z"}}));
    assert_eq!(code, "LimitExceeded");
    assert_eq!(tags(&mut h).as_object().unwrap().len(), 49);
    assert_eq!(tags(&mut h)["k0"], "v");
}

#[test]
fn invalid_value_wins_over_limit() {
    let mut h = new_queue();
    tag(&mut h, many(0..50));
    let code = h.err(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"extra": 1}}));
    assert_eq!(code, "InvalidParameterValue");
}

#[test]
fn tags_are_not_attributes_and_survive_purge_but_not_delete() {
    let mut h = new_queue();
    tag(&mut h, json!({"a": "1"}));
    let r = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert!(r["Attributes"].get("Tags").is_none());
    // CreateQueue rule 4 ignores tags.
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    h.ok(json!({"Command": "PurgeQueue", "QueueName": "q"}));
    assert_eq!(tags(&mut h), json!({"a": "1"}));
    h.ok(json!({"Command": "DeleteQueue", "QueueName": "q"}));
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    assert_eq!(tags(&mut h), json!({}));
}
