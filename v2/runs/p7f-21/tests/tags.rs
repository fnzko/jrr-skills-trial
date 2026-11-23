mod common;

use common::Harness;
use serde_json::{Value, json};

fn setup() -> Harness {
    let mut h = Harness::new();
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    h
}

fn tags(h: &mut Harness) -> Value {
    h.ok(json!({"Command": "ListQueueTags", "QueueName": "q"}))["Tags"].clone()
}

fn tag(h: &mut Harness, t: Value) {
    h.ok(json!({"Command": "TagQueue", "QueueName": "q", "Tags": t}));
}

#[test]
fn starts_empty_and_results_have_no_members() {
    let mut h = setup();
    assert_eq!(tags(&mut h), json!({}));
    assert_eq!(
        h.ok(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {}})),
        json!({"ok": true})
    );
    assert_eq!(
        h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": []})),
        json!({"ok": true})
    );
}

#[test]
fn tag_sets_and_replaces() {
    let mut h = setup();
    tag(&mut h, json!({"a": "1", "B": "", "k-_9": "x"}));
    assert_eq!(tags(&mut h), json!({"a": "1", "B": "", "k-_9": "x"}));
    tag(&mut h, json!({"a": "2"}));
    assert_eq!(tags(&mut h), json!({"a": "2", "B": "", "k-_9": "x"}));
}

#[test]
fn keys_are_case_sensitive() {
    let mut h = setup();
    tag(&mut h, json!({"a": "1", "A": "2"}));
    assert_eq!(tags(&mut h), json!({"a": "1", "A": "2"}));
}

#[test]
fn untag_removes_and_ignores_unknown() {
    let mut h = setup();
    tag(&mut h, json!({"a": "1", "b": "2"}));
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["a", "nope"]}));
    assert_eq!(tags(&mut h), json!({"b": "2"}));
}

#[test]
fn value_checks() {
    let mut h = setup();
    for bad in [json!({"": "x"}), json!({"a b": "x"}), json!({"é": "x"}),
        json!({"a": 1}), json!({"a": null}), json!({"a": "x".repeat(257)}),
        json!({"a": "é".repeat(129)}), json!({"k".repeat(129): "x"})] {
        assert_eq!(
            h.err(json!({"Command": "TagQueue", "QueueName": "q", "Tags": bad})),
            "InvalidParameterValue",
            "{bad}"
        );
    }
    tag(&mut h, json!({"k".repeat(128): "x".repeat(256)}));
    tag(&mut h, json!({"b": "é".repeat(128)}));
    // an invalid pair fails the whole command
    h.err(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"c": "1", "d": 2}}));
    assert!(tags(&mut h).get("c").is_none());
}

#[test]
fn untag_value_checks() {
    let mut h = setup();
    for bad in [json!(["a b"]), json!([1]), json!(["ok", null]), json!("a"), json!({})] {
        assert_eq!(
            h.err(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": bad})),
            "InvalidParameterValue",
            "{bad}"
        );
    }
}

#[test]
fn missing_and_unknown_queue() {
    let mut h = setup();
    assert_eq!(h.err(json!({"Command": "TagQueue", "QueueName": "q"})), "MissingParameter");
    assert_eq!(h.err(json!({"Command": "TagQueue", "Tags": {}})), "MissingParameter");
    assert_eq!(h.err(json!({"Command": "UntagQueue", "QueueName": "q"})), "MissingParameter");
    assert_eq!(h.err(json!({"Command": "ListQueueTags"})), "MissingParameter");
    // value errors come before the lookup
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "nope", "Tags": {"a b": "x"}})),
        "InvalidParameterValue"
    );
    for c in [
        json!({"Command": "TagQueue", "QueueName": "nope", "Tags": {}}),
        json!({"Command": "UntagQueue", "QueueName": "nope", "TagKeys": []}),
        json!({"Command": "ListQueueTags", "QueueName": "nope"}),
    ] {
        assert_eq!(h.err(c), "QueueDoesNotExist");
    }
}

#[test]
fn at_most_50_tags() {
    let mut h = setup();
    let many = |n: usize| -> Value {
        (0..n).map(|i| (format!("k{i}"), json!("v"))).collect::<serde_json::Map<_, _>>().into()
    };
    tag(&mut h, many(50));
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"extra": "v"}})),
        "LimitExceeded"
    );
    // replacing existing keys does not add
    tag(&mut h, json!({"k0": "new", "k49": "new"}));
    // removing makes room; a failed command changes nothing
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["k0"]}));
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"x": "1", "y": "2"}})),
        "LimitExceeded"
    );
    assert_eq!(tags(&mut h).as_object().unwrap().len(), 49);
    tag(&mut h, json!({"x": "1"}));
    assert_eq!(tags(&mut h).as_object().unwrap().len(), 50);
    assert_eq!(h.err(json!({"Command": "TagQueue", "QueueName": "q", "Tags": many(51)})), "LimitExceeded");
}

#[test]
fn tags_are_not_attributes_and_survive_purge() {
    let mut h = setup();
    tag(&mut h, json!({"a": "1"}));
    let r = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert!(r["Attributes"].get("Tags").is_none());
    assert!(r.get("Tags").is_none());
    // CreateQueue again with the same attributes still succeeds
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    h.ok(json!({"Command": "SendMessage", "QueueName": "q", "Body": "x"}));
    h.ok(json!({"Command": "PurgeQueue", "QueueName": "q"}));
    assert_eq!(tags(&mut h), json!({"a": "1"}));
}

#[test]
fn delete_queue_drops_tags() {
    let mut h = setup();
    tag(&mut h, json!({"a": "1"}));
    h.ok(json!({"Command": "DeleteQueue", "QueueName": "q"}));
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    assert_eq!(tags(&mut h), json!({}));
}
