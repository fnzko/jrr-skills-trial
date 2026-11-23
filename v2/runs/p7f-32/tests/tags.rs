mod common;

use common::Harness;
use serde_json::{Value, json};

fn tag(queue: &str, tags: Value) -> Value {
    json!({"Command": "TagQueue", "QueueName": queue, "Tags": tags})
}

fn tags_of(h: &mut Harness, queue: &str) -> Value {
    h.ok(json!({"Command": "ListQueueTags", "QueueName": queue}))["Tags"].clone()
}

fn harness_with_queue() -> Harness {
    let mut h = Harness::new();
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    h
}

#[test]
fn new_queue_has_no_tags() {
    let mut h = harness_with_queue();
    assert_eq!(tags_of(&mut h, "q"), json!({}));
}

#[test]
fn tagging_sets_and_replaces_values() {
    let mut h = harness_with_queue();
    h.ok(tag("q", json!({"env": "dev", "Team": "", "n": "1"})));
    h.ok(tag("q", json!({"env": "prod"})));
    assert_eq!(
        tags_of(&mut h, "q"),
        json!({"env": "prod", "Team": "", "n": "1"})
    );
}

#[test]
fn untagging_removes_known_keys_and_ignores_others() {
    let mut h = harness_with_queue();
    h.ok(tag("q", json!({"a": "1", "b": "2"})));
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["a", "zzz", "A"]}));
    assert_eq!(tags_of(&mut h, "q"), json!({"b": "2"}));
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": []}));
    assert_eq!(tags_of(&mut h, "q"), json!({"b": "2"}));
}

#[test]
fn keys_are_case_sensitive() {
    let mut h = harness_with_queue();
    h.ok(tag("q", json!({"a": "1", "A": "2"})));
    assert_eq!(tags_of(&mut h, "q"), json!({"a": "1", "A": "2"}));
}

#[test]
fn invalid_keys_and_values_are_rejected_without_effect() {
    let mut h = harness_with_queue();
    let long_key = "k".repeat(129);
    let long_value = "v".repeat(257);
    for bad in [
        json!({"ok": "1", "": "x"}),
        json!({"ok": "1", "a b": "x"}),
        json!({"ok": "1", "a.b": "x"}),
        json!({"ok": "1", long_key: "x"}),
        json!({"ok": "1", "k": long_value}),
        json!({"ok": "1", "k": 5}),
        json!({"ok": "1", "k": null}),
    ] {
        assert_eq!(h.err(tag("q", bad)), "InvalidParameterValue");
    }
    assert_eq!(tags_of(&mut h, "q"), json!({}));
    h.ok(tag(
        "q",
        json!({"k".repeat(128): "é".repeat(128), "e": "v".repeat(256)}),
    ));
    assert_eq!(
        h.err(tag("q", json!({"k": "é".repeat(129)}))),
        "InvalidParameterValue"
    );
}

#[test]
fn untag_rejects_bad_key_elements() {
    let mut h = harness_with_queue();
    h.ok(tag("q", json!({"a": "1"})));
    for bad in [
        json!(["a", 1]),
        json!(["a", ""]),
        json!(["a", "b c"]),
        json!("a"),
    ] {
        assert_eq!(
            h.err(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": bad})),
            "InvalidParameterValue"
        );
    }
    assert_eq!(tags_of(&mut h, "q"), json!({"a": "1"}));
}

#[test]
fn at_most_fifty_tags() {
    let mut h = harness_with_queue();
    let fifty: serde_json::Map<String, Value> =
        (0..50).map(|i| (format!("k{i}"), json!("v"))).collect();
    h.ok(tag("q", Value::Object(fifty)));
    h.ok(tag("q", json!({"k0": "changed"})));
    assert_eq!(h.err(tag("q", json!({"extra": "v"}))), "LimitExceeded");
    let tags = tags_of(&mut h, "q");
    assert_eq!(tags.as_object().unwrap().len(), 50);
    assert_eq!(tags["k0"], "changed");
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["k1"]}));
    h.ok(tag("q", json!({"extra": "v"})));
    assert_eq!(tags_of(&mut h, "q")["extra"], "v");
}

#[test]
fn a_tag_set_over_the_limit_changes_nothing() {
    let mut h = harness_with_queue();
    let fifty: serde_json::Map<String, Value> =
        (0..49).map(|i| (format!("k{i}"), json!("v"))).collect();
    h.ok(tag("q", Value::Object(fifty)));
    assert_eq!(
        h.err(tag("q", json!({"x": "1", "y": "2"}))),
        "LimitExceeded"
    );
    assert_eq!(tags_of(&mut h, "q").as_object().unwrap().len(), 49);
}

#[test]
fn parameter_errors_follow_the_check_order() {
    let mut h = harness_with_queue();
    for command in ["TagQueue", "UntagQueue", "ListQueueTags"] {
        assert_eq!(h.err(json!({"Command": command})), "MissingParameter");
        assert_eq!(
            h.err(json!({"Command": command, "QueueName": "nope", "Tags": {}, "TagKeys": []})),
            "QueueDoesNotExist"
        );
    }
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "q"})),
        "MissingParameter"
    );
    assert_eq!(
        h.err(json!({"Command": "UntagQueue", "QueueName": "q"})),
        "MissingParameter"
    );
    assert_eq!(h.err(tag("q", json!([]))), "InvalidParameterValue");
    assert_eq!(
        h.err(tag("nope", json!({"bad key": "x"}))),
        "InvalidParameterValue"
    );
    assert_eq!(h.err(tag("nope", json!({"k": "v"}))), "QueueDoesNotExist");
    assert_eq!(
        h.err(json!({"Command": "ListQueueTags", "QueueName": 5})),
        "InvalidParameterValue"
    );
}

#[test]
fn tags_are_not_attributes_and_survive_purge_but_not_delete() {
    let mut h = harness_with_queue();
    h.ok(tag("q", json!({"a": "1"})));
    let attrs = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert!(attrs["Attributes"].get("a").is_none());
    assert!(attrs["Attributes"].get("Tags").is_none());
    h.ok(json!({"Command": "SendMessage", "QueueName": "q", "Body": "x"}));
    h.ok(json!({"Command": "PurgeQueue", "QueueName": "q"}));
    assert_eq!(tags_of(&mut h, "q"), json!({"a": "1"}));
    h.ok(json!({"Command": "DeleteQueue", "QueueName": "q"}));
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    assert_eq!(tags_of(&mut h, "q"), json!({}));
}
