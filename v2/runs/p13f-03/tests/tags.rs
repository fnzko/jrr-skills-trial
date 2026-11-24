mod common;

use common::Harness;
use serde_json::{Value, json};

fn queue() -> Harness {
    let mut h = Harness::new();
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    h
}

fn tags(h: &mut Harness) -> Value {
    h.ok(json!({"Command": "ListQueueTags", "QueueName": "q"}))["Tags"].clone()
}

fn tag(h: &mut Harness, tags: Value) -> String {
    h.err(json!({"Command": "TagQueue", "QueueName": "q", "Tags": tags}))
}

#[test]
fn new_queue_has_no_tags() {
    let mut h = queue();
    assert_eq!(tags(&mut h), json!({}));
}

#[test]
fn tag_sets_and_replaces_values() {
    let mut h = queue();
    h.ok(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"a": "1", "B": ""}}));
    h.ok(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"a": "2", "b": "x"}}));
    assert_eq!(tags(&mut h), json!({"a": "2", "B": "", "b": "x"}));
    h.ok(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {}}));
    assert_eq!(tags(&mut h), json!({"a": "2", "B": "", "b": "x"}));
}

#[test]
fn untag_removes_given_keys_and_ignores_unknown_ones() {
    let mut h = queue();
    h.ok(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"a": "1", "b": "2"}}));
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["a", "zzz", "A"]}));
    assert_eq!(tags(&mut h), json!({"b": "2"}));
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": []}));
    assert_eq!(tags(&mut h), json!({"b": "2"}));
}

#[test]
fn invalid_keys_and_values_change_nothing() {
    let mut h = queue();
    let long_key = "k".repeat(129);
    let long_value = "v".repeat(257);
    for bad in [
        json!({"ok": "1", "": "x"}),
        json!({"ok": "1", "a b": "x"}),
        json!({"ok": "1", "é": "x"}),
        json!({"ok": "1", long_key: "x"}),
        json!({"ok": "1", "k": 5}),
        json!({"ok": "1", "k": null}),
        json!({"ok": "1", "k": long_value}),
        json!({"ok": "1", "k": "é".repeat(129)}),
    ] {
        assert_eq!(tag(&mut h, bad), "InvalidParameterValue");
    }
    assert_eq!(tags(&mut h), json!({}));
    h.ok(json!({"Command": "TagQueue", "QueueName": "q",
                "Tags": {"k".repeat(128): "é".repeat(128)}}));
}

#[test]
fn untag_rejects_bad_keys() {
    let mut h = queue();
    for bad in [json!(["a", 1]), json!([null]), json!(["a b"]), json!("a")] {
        let code = h.err(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": bad}));
        assert_eq!(code, "InvalidParameterValue");
    }
}

#[test]
fn parameter_check_order() {
    let mut h = queue();
    let tag_cmd = |q: Value, t: Value| json!({"Command": "TagQueue", "QueueName": q, "Tags": t});
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "Tags": {}})),
        "MissingParameter"
    );
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "q"})),
        "MissingParameter"
    );
    assert_eq!(h.err(tag_cmd(json!(1), json!({}))), "InvalidParameterValue");
    assert_eq!(
        h.err(tag_cmd(json!("q"), json!([]))),
        "InvalidParameterValue"
    );
    assert_eq!(
        h.err(tag_cmd(json!("nope"), json!({"a b": "x"}))),
        "InvalidParameterValue"
    );
    assert_eq!(
        h.err(tag_cmd(json!("nope"), json!({}))),
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
    assert_eq!(
        h.err(json!({"Command": "ListQueueTags"})),
        "MissingParameter"
    );
}

#[test]
fn at_most_fifty_tags() {
    let mut h = queue();
    let fifty: serde_json::Map<String, Value> =
        (0..50).map(|i| (format!("k{i}"), json!("v"))).collect();
    h.ok(json!({"Command": "TagQueue", "QueueName": "q", "Tags": fifty}));
    assert_eq!(tag(&mut h, json!({"extra": "v"})), "LimitExceeded");
    // Replacing existing keys is not growth.
    h.ok(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"k0": "new"}}));
    assert_eq!(tags(&mut h).as_object().unwrap().len(), 50);
    assert_eq!(tags(&mut h)["k0"], "new");
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["k1"]}));
    h.ok(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"extra": "v"}}));
}

#[test]
fn over_limit_batch_of_tags_changes_nothing() {
    let mut h = queue();
    let forty: serde_json::Map<String, Value> =
        (0..40).map(|i| (format!("k{i}"), json!("v"))).collect();
    h.ok(json!({"Command": "TagQueue", "QueueName": "q", "Tags": forty}));
    let eleven_new: serde_json::Map<String, Value> =
        (0..11).map(|i| (format!("n{i}"), json!("v"))).collect();
    assert_eq!(tag(&mut h, Value::Object(eleven_new)), "LimitExceeded");
    assert_eq!(tags(&mut h).as_object().unwrap().len(), 40);
}

#[test]
fn tags_are_not_attributes_and_survive_purge_and_create() {
    let mut h = queue();
    h.ok(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"a": "1"}}));
    let attrs = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(attrs["Attributes"].get("Tags"), None);
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    h.ok(json!({"Command": "PurgeQueue", "QueueName": "q"}));
    assert_eq!(tags(&mut h), json!({"a": "1"}));
}

#[test]
fn delete_queue_drops_tags() {
    let mut h = queue();
    h.ok(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"a": "1"}}));
    h.ok(json!({"Command": "DeleteQueue", "QueueName": "q"}));
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    assert_eq!(tags(&mut h), json!({}));
}
