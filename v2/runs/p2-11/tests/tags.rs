mod common;

use common::Harness;
use serde_json::{Value, json};

fn harness_with_queue() -> Harness {
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
fn a_new_queue_has_no_tags() {
    let mut h = harness_with_queue();
    assert_eq!(
        h.ok(json!({"Command": "ListQueueTags", "QueueName": "q"})),
        json!({"ok": true, "Tags": {}})
    );
}

#[test]
fn tagging_sets_and_replaces_values_and_untagging_removes_them() {
    let mut h = harness_with_queue();
    assert_eq!(
        h.ok(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"a": "1", "B": ""}})),
        json!({"ok": true})
    );
    assert_eq!(tags(&mut h), json!({"a": "1", "B": ""}));
    tag(&mut h, json!({"a": "2", "c": "3"}));
    assert_eq!(tags(&mut h), json!({"a": "2", "B": "", "c": "3"}));
    tag(&mut h, json!({}));
    assert_eq!(tags(&mut h), json!({"a": "2", "B": "", "c": "3"}));
    assert_eq!(
        h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["a", "missing", "b"]})),
        json!({"ok": true})
    );
    // Keys are case-sensitive: "b" did not remove "B".
    assert_eq!(tags(&mut h), json!({"B": "", "c": "3"}));
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": []}));
    assert_eq!(tags(&mut h), json!({"B": "", "c": "3"}));
}

#[test]
fn tag_key_and_value_rules() {
    let mut h = harness_with_queue();
    let ok_key = "k".repeat(128);
    let ok_value = "é".repeat(128); // 256 bytes
    tag(&mut h, json!({ok_key.clone(): ok_value.clone()}));
    assert_eq!(tags(&mut h), json!({ok_key: ok_value}));

    for bad_tags in [
        json!({"": "v"}),
        json!({"k".repeat(129): "v"}),
        json!({"a b": "v"}),
        json!({"é": "v"}),
        json!({"k": 1}),
        json!({"k": null}),
        json!({"k": "é".repeat(128) + "a"}),
        // One bad tag rejects the whole command.
        json!({"good": "v", "bad key": "v"}),
        json!([]),
        json!("x"),
    ] {
        assert_eq!(
            h.err(json!({"Command": "TagQueue", "QueueName": "q", "Tags": bad_tags})),
            "InvalidParameterValue",
            "{bad_tags}"
        );
    }
    assert_eq!(tags(&mut h).as_object().unwrap().len(), 1);

    for bad_keys in [
        json!(["a b"]),
        json!([1]),
        json!([null]),
        json!(["a", ""]),
        json!("a"),
        json!({}),
    ] {
        assert_eq!(
            h.err(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": bad_keys})),
            "InvalidParameterValue",
            "{bad_keys}"
        );
    }
}

#[test]
fn at_most_fifty_tags() {
    let mut h = harness_with_queue();
    let fifty: serde_json::Map<String, Value> =
        (0..50).map(|i| (format!("k{i}"), json!("v"))).collect();
    tag(&mut h, Value::Object(fifty));
    // Replacing an existing key adds no tag.
    tag(&mut h, json!({"k0": "new"}));
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"extra": "v"}})),
        "LimitExceeded"
    );
    // A failed command changes nothing, even the replacement that was fine.
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"k1": "changed", "extra": "v"}})),
        "LimitExceeded"
    );
    let listed = tags(&mut h);
    assert_eq!(listed.as_object().unwrap().len(), 50);
    assert_eq!(listed["k0"], "new");
    assert_eq!(listed["k1"], "v");
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["k2"]}));
    tag(&mut h, json!({"extra": "v"}));
}

#[test]
fn check_order() {
    let mut h = harness_with_queue();
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
    // Values are checked before the queue lookup.
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "nope", "Tags": {"a b": "v"}})),
        "InvalidParameterValue"
    );
    for command in [
        json!({"Command": "TagQueue", "QueueName": "nope", "Tags": {}}),
        json!({"Command": "UntagQueue", "QueueName": "nope", "TagKeys": []}),
        json!({"Command": "ListQueueTags", "QueueName": "nope"}),
    ] {
        assert_eq!(h.err(command), "QueueDoesNotExist");
    }
}

#[test]
fn tags_are_not_attributes_and_survive_purge_but_not_delete() {
    let mut h = harness_with_queue();
    tag(&mut h, json!({"a": "1"}));
    let attributes =
        h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}))["Attributes"].clone();
    assert!(attributes.get("a").is_none() && attributes.get("Tags").is_none());
    // CreateQueue rule 4 ignores tags.
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    h.ok(json!({"Command": "PurgeQueue", "QueueName": "q"}));
    assert_eq!(tags(&mut h), json!({"a": "1"}));
    h.ok(json!({"Command": "DeleteQueue", "QueueName": "q"}));
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    assert_eq!(tags(&mut h), json!({}));
}
