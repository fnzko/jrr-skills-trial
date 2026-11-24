mod common;

use common::Harness;
use serde_json::{Value, json};

fn harness_with_queue() -> Harness {
    let mut h = Harness::new();
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    h
}

fn tag(h: &mut Harness, tags: Value) -> Value {
    h.ok(json!({"Command": "TagQueue", "QueueName": "q", "Tags": tags}))
}

fn list(h: &mut Harness) -> Value {
    h.ok(json!({"Command": "ListQueueTags", "QueueName": "q"}))
}

#[test]
fn new_queue_has_no_tags_and_responses_have_only_listed_members() {
    let mut h = harness_with_queue();
    assert_eq!(list(&mut h), json!({"ok": true, "Tags": {}}));
    assert_eq!(tag(&mut h, json!({})), json!({"ok": true}));
    assert_eq!(
        h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": []})),
        json!({"ok": true})
    );
}

#[test]
fn tag_sets_and_replaces_values() {
    let mut h = harness_with_queue();
    tag(&mut h, json!({"env": "dev", "team": "", "Env": "x"}));
    tag(&mut h, json!({"env": "prod"}));
    assert_eq!(
        list(&mut h)["Tags"],
        json!({"env": "prod", "team": "", "Env": "x"})
    );
}

#[test]
fn untag_removes_given_keys_and_ignores_unknown_ones() {
    let mut h = harness_with_queue();
    tag(&mut h, json!({"a": "1", "b": "2", "c": "3"}));
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["a", "c", "zzz", "a"]}));
    assert_eq!(list(&mut h)["Tags"], json!({"b": "2"}));
}

#[test]
fn tag_values_are_limited_in_utf8_bytes() {
    let mut h = harness_with_queue();
    tag(
        &mut h,
        json!({"ascii": "a".repeat(256), "wide": "é".repeat(128)}),
    );
    for value in [
        json!("a".repeat(257)),
        json!("é".repeat(129)),
        json!(5),
        json!(null),
    ] {
        assert_eq!(
            h.err(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"k": value}})),
            "InvalidParameterValue"
        );
    }
}

#[test]
fn tag_keys_follow_the_identifier_rule() {
    let mut h = harness_with_queue();
    tag(&mut h, json!({"k".repeat(128): "v", "A_z-9": "v"}));
    for key in ["", "has space", "dot.ted", "é", &"k".repeat(129)] {
        assert_eq!(
            h.err(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {key: "v"}})),
            "InvalidParameterValue",
            "{key:?}"
        );
    }
    for bad in [json!([5]), json!([null]), json!([""]), json!(["a b"])] {
        assert_eq!(
            h.err(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": bad})),
            "InvalidParameterValue"
        );
    }
    assert_eq!(
        h.err(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": {}})),
        "InvalidParameterValue"
    );
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "q", "Tags": []})),
        "InvalidParameterValue"
    );
}

#[test]
fn check_order_of_tag_commands() {
    let mut h = Harness::new();
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "q"})),
        "MissingParameter"
    );
    assert_eq!(
        h.err(json!({"Command": "UntagQueue", "Tags": {}})),
        "MissingParameter"
    );
    assert_eq!(
        h.err(json!({"Command": "ListQueueTags"})),
        "MissingParameter"
    );
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"bad key": "v"}})),
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
    assert_eq!(
        h.err(json!({"Command": "ListQueueTags", "QueueName": "q"})),
        "QueueDoesNotExist"
    );
}

#[test]
fn at_most_fifty_tags_and_a_failed_command_changes_nothing() {
    let mut h = harness_with_queue();
    let keys = |range: std::ops::Range<u32>| -> Value {
        range
            .map(|i| (format!("k{i}"), json!("v")))
            .collect::<serde_json::Map<_, _>>()
            .into()
    };
    tag(&mut h, keys(0..50));
    // Replacing existing keys is fine at the limit.
    tag(&mut h, json!({"k0": "new", "k49": "new"}));
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"k0": "x", "extra": "x"}})),
        "LimitExceeded"
    );
    let tags = list(&mut h)["Tags"].clone();
    assert_eq!(tags.as_object().unwrap().len(), 50);
    assert_eq!(tags["k0"], json!("new"));
    assert!(tags.get("extra").is_none());
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["k1"]}));
    tag(&mut h, json!({"extra": "x"}));
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "q", "Tags": keys(100..151)})),
        "LimitExceeded"
    );
    // The size rule comes before the limit.
    assert_eq!(
        h.err(
            json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"a": 1, "b": "x", "c": "x"}})
        ),
        "InvalidParameterValue"
    );
}

#[test]
fn tags_are_not_attributes_and_survive_purge_but_not_delete() {
    let mut h = harness_with_queue();
    tag(&mut h, json!({"a": "1"}));
    let attrs = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert!(attrs["Attributes"].get("Tags").is_none());
    assert!(attrs.get("Tags").is_none());
    // CreateQueue rule 4 ignores tags.
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    h.ok(json!({"Command": "PurgeQueue", "QueueName": "q"}));
    assert_eq!(list(&mut h)["Tags"], json!({"a": "1"}));
    h.ok(json!({"Command": "DeleteQueue", "QueueName": "q"}));
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    assert_eq!(list(&mut h)["Tags"], json!({}));
}
