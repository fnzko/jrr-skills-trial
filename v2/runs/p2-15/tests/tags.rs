mod common;

use common::Harness;
use serde_json::{Value, json};

fn harness() -> Harness {
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
fn tags_start_empty_and_results_have_no_members() {
    let mut h = harness();
    assert_eq!(list(&mut h), json!({}));
    assert_eq!(tag(&mut h, json!({})), json!({"ok": true}));
    assert_eq!(
        h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": []})),
        json!({"ok": true})
    );
}

#[test]
fn tag_sets_and_replaces_and_keys_are_case_sensitive() {
    let mut h = harness();
    tag(&mut h, json!({"a": "1", "A": "", "b-_9": "ü"}));
    tag(&mut h, json!({"a": "2"}));
    assert_eq!(list(&mut h), json!({"a": "2", "A": "", "b-_9": "ü"}));
}

#[test]
fn untag_removes_given_keys_and_ignores_unknown_ones() {
    let mut h = harness();
    tag(&mut h, json!({"a": "1", "b": "2", "c": "3"}));
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["a", "zzz", "c", "a"]}));
    assert_eq!(list(&mut h), json!({"b": "2"}));
}

#[test]
fn tag_value_and_key_validation() {
    let mut h = harness();
    let t = |tags: Value| json!({"Command": "TagQueue", "QueueName": "q", "Tags": tags});
    assert_eq!(h.err(t(json!({"": "v"}))), "InvalidParameterValue");
    assert_eq!(h.err(t(json!({"a b": "v"}))), "InvalidParameterValue");
    assert_eq!(h.err(t(json!({"é": "v"}))), "InvalidParameterValue");
    assert_eq!(h.err(t(json!({"k": 1}))), "InvalidParameterValue");
    assert_eq!(h.err(t(json!({"k": null}))), "InvalidParameterValue");
    assert_eq!(h.err(t(json!([]))), "InvalidParameterValue");
    assert_eq!(
        h.err(t(json!({"x".repeat(129): "v"}))),
        "InvalidParameterValue"
    );
    assert_eq!(
        h.err(t(json!({"k": "x".repeat(257)}))),
        "InvalidParameterValue"
    );
    // 129 bytes of 2-byte characters is 258 bytes: too long, though only 129 characters.
    assert_eq!(
        h.err(t(json!({"k": "é".repeat(129)}))),
        "InvalidParameterValue"
    );
    // Valid edges.
    tag(&mut h, json!({"x".repeat(128): "y".repeat(256)}));
    tag(&mut h, json!({"k": "é".repeat(128)}));
    // A bad tag in a request changes nothing.
    assert_eq!(
        h.err(t(json!({"good": "1", "bad key": "2"}))),
        "InvalidParameterValue"
    );
    assert_eq!(
        list(&mut h),
        json!({"x".repeat(128): "y".repeat(256), "k": "é".repeat(128)})
    );
}

#[test]
fn untag_key_validation() {
    let mut h = harness();
    let u = |keys: Value| json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": keys});
    assert_eq!(h.err(u(json!("a"))), "InvalidParameterValue");
    assert_eq!(h.err(u(json!([1]))), "InvalidParameterValue");
    assert_eq!(h.err(u(json!(["a", "bad key"]))), "InvalidParameterValue");
    assert_eq!(h.err(u(json!([null]))), "InvalidParameterValue");
}

#[test]
fn at_most_fifty_tags() {
    let mut h = harness();
    let many = |range: std::ops::Range<u32>| -> Value {
        range
            .map(|i| (format!("k{i}"), json!("v")))
            .collect::<serde_json::Map<_, _>>()
            .into()
    };
    tag(&mut h, many(0..50));
    // Replacing existing keys stays within the limit.
    tag(&mut h, json!({"k0": "new"}));
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"extra": "v"}})),
        "LimitExceeded"
    );
    // A failed command changes nothing, even for its replacing members.
    assert_eq!(
        h.err(
            json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"extra": "v", "k1": "new"}})
        ),
        "LimitExceeded"
    );
    let tags = list(&mut h);
    assert_eq!(tags.as_object().unwrap().len(), 50);
    assert_eq!(tags["k0"], "new");
    assert_eq!(tags["k1"], "v");
    // Removing one makes room again.
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["k5"]}));
    tag(&mut h, json!({"extra": "v"}));
    // 51 members in one request on an empty queue.
    h.ok(json!({"Command": "CreateQueue", "QueueName": "e"}));
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "e", "Tags": many(0..51)})),
        "LimitExceeded"
    );
}

#[test]
fn check_order() {
    let mut h = harness();
    // Required parameters, then values, then lookup, then the limit.
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": 5})),
        "MissingParameter"
    );
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "Tags": null, "QueueName": "q"})),
        "MissingParameter"
    );
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "nope", "Tags": {"a b": "v"}})),
        "InvalidParameterValue"
    );
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "nope", "Tags": {}})),
        "QueueDoesNotExist"
    );
    assert_eq!(
        h.err(json!({"Command": "UntagQueue", "QueueName": "q"})),
        "MissingParameter"
    );
    assert_eq!(
        h.err(json!({"Command": "UntagQueue", "QueueName": "nope", "TagKeys": []})),
        "QueueDoesNotExist"
    );
    assert_eq!(
        h.err(json!({"Command": "ListQueueTags"})),
        "MissingParameter"
    );
    assert_eq!(
        h.err(json!({"Command": "ListQueueTags", "QueueName": "nope"})),
        "QueueDoesNotExist"
    );
}

#[test]
fn tags_are_not_attributes() {
    let mut h = Harness::new();
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": {"DelaySeconds": 3}}));
    let before = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    tag(&mut h, json!({"a": "1"}));
    assert_eq!(
        h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"})),
        before
    );
    // CreateQueue rule 4 ignores tags.
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": {"DelaySeconds": 3}}));
    assert_eq!(list(&mut h), json!({"a": "1"}));
}

#[test]
fn purge_keeps_tags_and_delete_drops_them() {
    let mut h = harness();
    tag(&mut h, json!({"a": "1"}));
    h.ok(json!({"Command": "PurgeQueue", "QueueName": "q"}));
    assert_eq!(list(&mut h), json!({"a": "1"}));
    h.ok(json!({"Command": "DeleteQueue", "QueueName": "q"}));
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    assert_eq!(list(&mut h), json!({}));
}

#[test]
fn tags_are_per_queue() {
    let mut h = harness();
    h.ok(json!({"Command": "CreateQueue", "QueueName": "other"}));
    tag(&mut h, json!({"a": "1"}));
    let other = h.ok(json!({"Command": "ListQueueTags", "QueueName": "other"}));
    assert_eq!(other["Tags"], json!({}));
}
