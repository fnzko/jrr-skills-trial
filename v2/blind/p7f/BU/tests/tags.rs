mod common;

use common::Harness;
use serde_json::{Value, json};

fn tag(queue: &str, tags: Value) -> Value {
    json!({"Command": "TagQueue", "QueueName": queue, "Tags": tags})
}

fn untag(queue: &str, keys: Value) -> Value {
    json!({"Command": "UntagQueue", "QueueName": queue, "TagKeys": keys})
}

fn tags_of(h: &mut Harness, queue: &str) -> Value {
    h.ok(json!({"Command": "ListQueueTags", "QueueName": queue}))["Tags"].clone()
}

fn queue_with_tags() -> Harness {
    let mut h = Harness::new();
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    h
}

#[test]
fn new_queue_has_no_tags_and_list_has_exact_shape() {
    let mut h = queue_with_tags();
    let r = h.ok(json!({"Command": "ListQueueTags", "QueueName": "q"}));
    assert_eq!(r, json!({"ok": true, "Tags": {}}));
}

#[test]
fn tag_sets_and_replaces_values() {
    let mut h = queue_with_tags();
    assert_eq!(
        h.ok(tag("q", json!({"env": "dev", "Team": ""}))),
        json!({"ok": true})
    );
    h.ok(tag("q", json!({"env": "prod", "other": "x"})));
    h.ok(tag("q", json!({})));
    assert_eq!(
        tags_of(&mut h, "q"),
        json!({"env": "prod", "Team": "", "other": "x"})
    );
}

#[test]
fn keys_are_case_sensitive() {
    let mut h = queue_with_tags();
    h.ok(tag("q", json!({"a": "1", "A": "2"})));
    assert_eq!(tags_of(&mut h, "q"), json!({"a": "1", "A": "2"}));
}

#[test]
fn tag_value_and_key_rules() {
    let mut h = queue_with_tags();
    h.ok(tag("q", json!({"k".repeat(128): "v".repeat(256)})));
    // 256 bytes counts bytes, not characters: 129 two-byte characters are too long.
    h.ok(tag("q", json!({"b": "é".repeat(128)})));
    let bad = [
        json!({"k".repeat(129): "v"}),
        json!({"": "v"}),
        json!({"a b": "v"}),
        json!({"é": "v"}),
        json!({"k": "v".repeat(257)}),
        json!({"k": "é".repeat(129)}),
        json!({"k": 1}),
        json!({"k": null}),
        json!({"ok": "v", "k": ["v"]}),
    ];
    for tags in bad {
        assert_eq!(
            h.err(tag("q", tags.clone())),
            "InvalidParameterValue",
            "{tags}"
        );
    }
    assert_eq!(h.err(tag("q", json!([]))), "InvalidParameterValue");
    assert_eq!(h.err(tag("q", json!("x"))), "InvalidParameterValue");
    // A rejected command sets nothing, not even its valid members.
    assert_eq!(
        h.err(tag("q", json!({"fine": "v", "bad key": "v"}))),
        "InvalidParameterValue"
    );
    assert_eq!(tags_of(&mut h, "q").as_object().unwrap().len(), 2);
}

#[test]
fn tag_check_order() {
    let mut h = queue_with_tags();
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "q"})),
        "MissingParameter"
    );
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "Tags": {}})),
        "MissingParameter"
    );
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "q", "Tags": null})),
        "MissingParameter"
    );
    assert_eq!(
        h.err(tag("missing", json!({"bad key": "v"}))),
        "InvalidParameterValue"
    );
    assert_eq!(h.err(tag("missing", json!({}))), "QueueDoesNotExist");
    assert_eq!(
        h.err(json!({"Command": "UntagQueue", "QueueName": "q"})),
        "MissingParameter"
    );
    assert_eq!(h.err(untag("missing", json!([]))), "QueueDoesNotExist");
    assert_eq!(
        h.err(json!({"Command": "ListQueueTags"})),
        "MissingParameter"
    );
    assert_eq!(
        h.err(json!({"Command": "ListQueueTags", "QueueName": "missing"})),
        "QueueDoesNotExist"
    );
}

#[test]
fn at_most_fifty_tags() {
    let mut h = queue_with_tags();
    let fifty: serde_json::Map<String, Value> =
        (0..50).map(|i| (format!("k{i}"), json!("v"))).collect();
    h.ok(tag("q", Value::Object(fifty.clone())));
    // Replacing existing keys stays within the limit.
    h.ok(tag("q", json!({"k0": "new", "k49": "new"})));
    assert_eq!(h.err(tag("q", json!({"extra": "v"}))), "LimitExceeded");
    // Invalid values are reported before the limit.
    assert_eq!(
        h.err(tag("q", json!({"extra": 1}))),
        "InvalidParameterValue"
    );
    assert_eq!(
        h.err(tag("q", json!({"extra": "v", "k1": "w"}))),
        "LimitExceeded"
    );
    let tags = tags_of(&mut h, "q");
    assert_eq!(tags.as_object().unwrap().len(), 50);
    assert_eq!(tags["k1"], "v");
    assert_eq!(tags["k0"], "new");
    h.ok(untag("q", json!(["k0"])));
    h.ok(tag("q", json!({"extra": "v"})));
    assert_eq!(tags_of(&mut h, "q").as_object().unwrap().len(), 50);
}

#[test]
fn over_the_limit_in_one_command_changes_nothing() {
    let mut h = queue_with_tags();
    let fifty_one: serde_json::Map<String, Value> =
        (0..51).map(|i| (format!("k{i}"), json!("v"))).collect();
    assert_eq!(h.err(tag("q", Value::Object(fifty_one))), "LimitExceeded");
    assert_eq!(tags_of(&mut h, "q"), json!({}));
}

#[test]
fn untag_removes_listed_keys_and_ignores_others() {
    let mut h = queue_with_tags();
    h.ok(tag("q", json!({"a": "1", "b": "2", "c": "3"})));
    assert_eq!(
        h.ok(untag("q", json!(["a", "zzz", "a", "B"]))),
        json!({"ok": true})
    );
    assert_eq!(tags_of(&mut h, "q"), json!({"b": "2", "c": "3"}));
    h.ok(untag("q", json!([])));
    assert_eq!(tags_of(&mut h, "q"), json!({"b": "2", "c": "3"}));
    for keys in [
        json!({}),
        json!("a"),
        json!([1]),
        json!([null]),
        json!(["a b"]),
        json!([""]),
        json!(["x".repeat(129)]),
    ] {
        assert_eq!(
            h.err(untag("q", keys.clone())),
            "InvalidParameterValue",
            "{keys}"
        );
    }
    // A rejected command removes nothing, not even its valid keys.
    assert_eq!(h.err(untag("q", json!(["b", 1]))), "InvalidParameterValue");
    assert_eq!(tags_of(&mut h, "q"), json!({"b": "2", "c": "3"}));
}

#[test]
fn tags_are_not_attributes_and_survive_purge_and_attribute_changes() {
    let mut h = queue_with_tags();
    let before = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    h.ok(tag("q", json!({"a": "1"})));
    assert_eq!(
        h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"})),
        before
    );
    h.ok(json!({"Command": "SendMessage", "QueueName": "q", "Body": "x"}));
    h.ok(json!({"Command": "PurgeQueue", "QueueName": "q"}));
    h.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"DelaySeconds": 3}}));
    assert_eq!(tags_of(&mut h, "q"), json!({"a": "1"}));
    // CreateQueue equality ignores tags.
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": {"DelaySeconds": 3}}));
    assert_eq!(tags_of(&mut h, "q"), json!({"a": "1"}));
}

#[test]
fn tags_disappear_with_the_queue_and_are_per_queue() {
    let mut h = queue_with_tags();
    h.ok(json!({"Command": "CreateQueue", "QueueName": "other"}));
    h.ok(tag("q", json!({"a": "1"})));
    assert_eq!(tags_of(&mut h, "other"), json!({}));
    h.ok(json!({"Command": "DeleteQueue", "QueueName": "q"}));
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    assert_eq!(tags_of(&mut h, "q"), json!({}));
}
