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

fn tags_of(h: &mut Harness) -> Value {
    h.ok(json!({"Command": "ListQueueTags", "QueueName": "q"}))["Tags"].clone()
}

#[test]
fn new_queue_has_no_tags() {
    let mut h = harness_with_queue();
    assert_eq!(tags_of(&mut h), json!({}));
}

#[test]
fn tagging_sets_and_replaces_values() {
    let mut h = harness_with_queue();
    tag(&mut h, json!({"env": "dev", "team": ""}));
    tag(&mut h, json!({"env": "prod", "Env": "x"}));
    assert_eq!(
        tags_of(&mut h),
        json!({"env": "prod", "team": "", "Env": "x"})
    );
}

#[test]
fn empty_tags_object_is_accepted() {
    let mut h = harness_with_queue();
    tag(&mut h, json!({}));
    assert_eq!(tags_of(&mut h), json!({}));
}

#[test]
fn invalid_keys_and_values_are_rejected_and_change_nothing() {
    let mut h = harness_with_queue();
    let bad = [
        json!({"ok": "1", "bad key": "v"}),
        json!({"": "v"}),
        json!({[&"k".repeat(129)][0]: "v"}),
        json!({"ok": "1", "n": 5}),
        json!({"ok": "1", "n": null}),
        json!({"k": "a".repeat(257)}),
        json!("not an object"),
    ];
    for tags in bad {
        assert_eq!(
            h.err(json!({"Command": "TagQueue", "QueueName": "q", "Tags": tags})),
            "InvalidParameterValue",
            "{tags}"
        );
    }
    assert_eq!(tags_of(&mut h), json!({}));
}

#[test]
fn value_limit_is_256_utf8_bytes_and_key_limit_128_characters() {
    let mut h = harness_with_queue();
    let long_key = "k".repeat(128);
    tag(&mut h, json!({ long_key.clone(): "é".repeat(128) }));
    assert_eq!(tags_of(&mut h)[&long_key], "é".repeat(128));
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "q",
                     "Tags": {"k": "é".repeat(128) + "a"}})),
        "InvalidParameterValue"
    );
}

#[test]
fn fifty_tags_fit_and_the_fifty_first_does_not() {
    let mut h = harness_with_queue();
    let fifty: serde_json::Map<String, Value> =
        (0..50).map(|i| (format!("k{i}"), json!("v"))).collect();
    tag(&mut h, Value::Object(fifty));
    tag(&mut h, json!({"k0": "changed"}));
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"extra": "v"}})),
        "LimitExceeded"
    );
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "q",
                     "Tags": {"k0": "again", "extra": "v"}})),
        "LimitExceeded"
    );
    let tags = tags_of(&mut h);
    assert_eq!(tags.as_object().unwrap().len(), 50);
    assert_eq!(tags["k0"], "changed");
}

#[test]
fn untag_removes_given_keys_and_ignores_unknown_ones() {
    let mut h = harness_with_queue();
    tag(&mut h, json!({"a": "1", "b": "2", "c": "3"}));
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["a", "zzz", "c"]}));
    assert_eq!(tags_of(&mut h), json!({"b": "2"}));
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": []}));
    assert_eq!(tags_of(&mut h), json!({"b": "2"}));
}

#[test]
fn untag_rejects_bad_elements_without_removing_anything() {
    let mut h = harness_with_queue();
    tag(&mut h, json!({"a": "1"}));
    for keys in [
        json!(["a", 5]),
        json!(["a", "bad key"]),
        json!("a"),
        json!(["a", null]),
    ] {
        assert_eq!(
            h.err(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": keys})),
            "InvalidParameterValue",
            "{keys}"
        );
    }
    assert_eq!(tags_of(&mut h), json!({"a": "1"}));
}

#[test]
fn missing_parameters_come_before_value_checks_and_lookup() {
    let mut h = Harness::new();
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "nope"})),
        "MissingParameter"
    );
    assert_eq!(
        h.err(json!({"Command": "UntagQueue", "Tags": 1})),
        "MissingParameter"
    );
    assert_eq!(
        h.err(json!({"Command": "ListQueueTags"})),
        "MissingParameter"
    );
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "nope", "Tags": 1})),
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
fn tags_are_not_attributes_and_survive_purge_but_not_delete() {
    let mut h = harness_with_queue();
    tag(&mut h, json!({"a": "1"}));
    let attrs = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(
        attrs["Attributes"],
        json!({
            "VisibilityTimeout": 30, "DelaySeconds": 0, "RetentionSeconds": 345600,
            "Fifo": false, "ContentBasedDeduplication": false, "MaxMessageBytes": 1024,
        })
    );
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    h.ok(json!({"Command": "PurgeQueue", "QueueName": "q"}));
    assert_eq!(tags_of(&mut h), json!({"a": "1"}));
    h.ok(json!({"Command": "DeleteQueue", "QueueName": "q"}));
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    assert_eq!(tags_of(&mut h), json!({}));
}
