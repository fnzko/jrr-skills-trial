mod common;

use common::Harness;
use serde_json::{Value, json};

fn harness_with_queue() -> Harness {
    let mut h = Harness::new();
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    h
}

fn tags(h: &mut Harness) -> Value {
    h.ok(json!({"Command": "ListQueueTags", "QueueName": "q"}))["Tags"].clone()
}

fn tag(h: &mut Harness, tags: Value) -> Value {
    h.run(json!({"Command": "TagQueue", "QueueName": "q", "Tags": tags}))
}

#[test]
fn new_queue_has_no_tags() {
    let mut h = harness_with_queue();
    assert_eq!(tags(&mut h), json!({}));
}

#[test]
fn tag_sets_replaces_and_keeps_others() {
    let mut h = harness_with_queue();
    assert_eq!(tag(&mut h, json!({"a": "1", "b": ""})), json!({"ok": true}));
    assert_eq!(
        tag(&mut h, json!({"a": "2", "A": "3"})),
        json!({"ok": true})
    );
    assert_eq!(tags(&mut h), json!({"a": "2", "b": "", "A": "3"}));
    assert_eq!(tag(&mut h, json!({})), json!({"ok": true}));
    assert_eq!(tags(&mut h), json!({"a": "2", "b": "", "A": "3"}));
}

#[test]
fn tag_value_errors() {
    let mut h = harness_with_queue();
    let long = "é".repeat(129);
    for bad in [
        json!({"": "v"}),
        json!({"a b": "v"}),
        json!({"k".repeat(129): "v"}),
        json!({"a": 1}),
        json!({"a": null}),
        json!({"a": long}),
    ] {
        assert_eq!(
            h.err(json!({"Command": "TagQueue", "QueueName": "q", "Tags": bad})),
            "InvalidParameterValue"
        );
    }
    h.ok(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"k".repeat(128): "x".repeat(256)}}));
    assert_eq!(tags(&mut h).as_object().unwrap().len(), 1);
}

#[test]
fn tag_check_order() {
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
        h.err(json!({"Command": "TagQueue", "QueueName": "q", "Tags": []})),
        "InvalidParameterValue"
    );
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "nope", "Tags": {"a": 1}})),
        "InvalidParameterValue"
    );
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "nope", "Tags": {}})),
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
    assert_eq!(
        h.err(json!({"Command": "UntagQueue", "QueueName": "nope", "TagKeys": []})),
        "QueueDoesNotExist"
    );
}

#[test]
fn fifty_tags_fit_and_fifty_one_do_not() {
    let mut h = harness_with_queue();
    let fifty: serde_json::Map<String, Value> =
        (0..50).map(|i| (format!("k{i}"), json!("v"))).collect();
    h.ok(json!({"Command": "TagQueue", "QueueName": "q", "Tags": fifty}));
    // Replacing an existing key adds nothing.
    h.ok(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"k0": "new"}}));
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"extra": "v"}})),
        "LimitExceeded"
    );
    // A failed command changes nothing, even for the keys that would fit.
    assert_eq!(h.err(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"k1": "changed", "extra": "v"}})), "LimitExceeded");
    let t = tags(&mut h);
    assert_eq!(t.as_object().unwrap().len(), 50);
    assert_eq!(t["k0"], "new");
    assert_eq!(t["k1"], "v");
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["k0"]}));
    h.ok(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"extra": "v"}}));
}

#[test]
fn untag_removes_listed_keys_and_ignores_unknown_ones() {
    let mut h = harness_with_queue();
    tag(&mut h, json!({"a": "1", "b": "2", "c": "3"}));
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["a", "zzz", "c"]}));
    assert_eq!(tags(&mut h), json!({"b": "2"}));
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": []}));
    assert_eq!(tags(&mut h), json!({"b": "2"}));
}

#[test]
fn untag_parameter_errors() {
    let mut h = harness_with_queue();
    assert_eq!(
        h.err(json!({"Command": "UntagQueue", "QueueName": "q"})),
        "MissingParameter"
    );
    assert_eq!(
        h.err(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": {}})),
        "InvalidParameterValue"
    );
    assert_eq!(
        h.err(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": [1]})),
        "InvalidParameterValue"
    );
    assert_eq!(
        h.err(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["a", "b c"]})),
        "InvalidParameterValue"
    );
}

#[test]
fn tags_are_not_attributes_and_survive_purge_but_not_delete() {
    let mut h = harness_with_queue();
    tag(&mut h, json!({"a": "1"}));
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    let r = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert!(r["Attributes"].get("Tags").is_none());
    h.ok(json!({"Command": "SendMessage", "QueueName": "q", "Body": "x"}));
    h.ok(json!({"Command": "PurgeQueue", "QueueName": "q"}));
    assert_eq!(tags(&mut h), json!({"a": "1"}));
    h.ok(json!({"Command": "DeleteQueue", "QueueName": "q"}));
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    assert_eq!(tags(&mut h), json!({}));
}
