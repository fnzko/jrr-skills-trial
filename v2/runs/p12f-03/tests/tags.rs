mod common;

use common::Harness;
use serde_json::json;

fn setup() -> Harness {
    let mut h = Harness::new();
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    h
}

fn tags(h: &mut Harness) -> serde_json::Value {
    h.ok(json!({"Command": "ListQueueTags", "QueueName": "q"}))["Tags"].clone()
}

#[test]
fn new_queue_has_no_tags() {
    let mut h = setup();
    assert_eq!(tags(&mut h), json!({}));
}

#[test]
fn tag_replaces_and_untag_removes() {
    let mut h = setup();
    h.ok(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"a": "1", "B": "", "c": "3"}}));
    h.ok(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"a": "9"}}));
    assert_eq!(tags(&mut h), json!({"a": "9", "B": "", "c": "3"}));
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["a", "zzz"]}));
    assert_eq!(tags(&mut h), json!({"B": "", "c": "3"}));
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": []}));
    h.ok(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {}}));
    assert_eq!(tags(&mut h), json!({"B": "", "c": "3"}));
}

#[test]
fn tag_value_limit_counts_utf8_bytes() {
    let mut h = setup();
    let tag = |v: String| json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"k": v}});
    h.ok(tag("é".repeat(128)));
    assert_eq!(h.err(tag("é".repeat(129))), "InvalidParameterValue");
    assert_eq!(h.err(tag("a".repeat(257))), "InvalidParameterValue");
}

#[test]
fn invalid_tag_arguments() {
    let mut h = setup();
    let tag = |t: serde_json::Value| json!({"Command": "TagQueue", "QueueName": "q", "Tags": t});
    let untag =
        |t: serde_json::Value| json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": t});
    assert_eq!(h.err(tag(json!({"": "x"}))), "InvalidParameterValue");
    assert_eq!(h.err(tag(json!({"a b": "x"}))), "InvalidParameterValue");
    assert_eq!(
        h.err(tag(json!({"k".repeat(129): "x"}))),
        "InvalidParameterValue"
    );
    assert_eq!(h.err(tag(json!({"k": 1}))), "InvalidParameterValue");
    assert_eq!(h.err(tag(json!({"k": null}))), "InvalidParameterValue");
    assert_eq!(h.err(tag(json!([]))), "InvalidParameterValue");
    assert_eq!(h.err(untag(json!({}))), "InvalidParameterValue");
    assert_eq!(h.err(untag(json!([1]))), "InvalidParameterValue");
    assert_eq!(h.err(untag(json!(["a/b"]))), "InvalidParameterValue");
    h.ok(tag(json!({"k".repeat(128): "x"})));
}

#[test]
fn check_order_for_tag_commands() {
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
        h.err(json!({"Command": "TagQueue", "QueueName": "nope", "Tags": {"": "x"}})),
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
fn at_most_fifty_tags() {
    let mut h = setup();
    let many = |range: std::ops::Range<u32>| {
        let tags: serde_json::Map<_, _> = range.map(|i| (format!("k{i}"), json!("v"))).collect();
        json!({"Command": "TagQueue", "QueueName": "q", "Tags": tags})
    };
    assert_eq!(h.err(many(0..51)), "LimitExceeded");
    assert_eq!(tags(&mut h), json!({}));
    h.ok(many(0..50));
    h.ok(many(0..50));
    assert_eq!(h.err(many(49..51)), "LimitExceeded");
    assert_eq!(tags(&mut h).as_object().unwrap().len(), 50);
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["k0"]}));
    h.ok(many(50..51));
    assert_eq!(tags(&mut h).as_object().unwrap().len(), 50);
}

#[test]
fn tags_are_not_attributes_and_survive_purge_but_not_delete() {
    let mut h = setup();
    h.ok(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"a": "1"}}));
    let attrs = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(attrs["Attributes"].get("Tags"), None);
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    h.ok(json!({"Command": "PurgeQueue", "QueueName": "q"}));
    assert_eq!(tags(&mut h), json!({"a": "1"}));
    h.ok(json!({"Command": "DeleteQueue", "QueueName": "q"}));
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    assert_eq!(tags(&mut h), json!({}));
}
