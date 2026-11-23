mod common;

use common::Harness;
use serde_json::{Value, json};

fn harness() -> Harness {
    let mut h = Harness::new();
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    h
}

fn tag(h: &mut Harness, tags: Value) -> Value {
    h.run(json!({"Command": "TagQueue", "QueueName": "q", "Tags": tags}))
}

fn tags(h: &mut Harness) -> Value {
    h.ok(json!({"Command": "ListQueueTags", "QueueName": "q"}))["Tags"].clone()
}

#[test]
fn tags_start_empty_and_are_set_and_replaced() {
    let mut h = harness();
    assert_eq!(tags(&mut h), json!({}));
    assert_eq!(tag(&mut h, json!({"a": "1", "B": ""})), json!({"ok": true}));
    assert_eq!(tags(&mut h), json!({"a": "1", "B": ""}));
    tag(&mut h, json!({"a": "2"}));
    assert_eq!(tags(&mut h), json!({"a": "2", "B": ""}));
    assert_eq!(tag(&mut h, json!({})), json!({"ok": true}));
    assert_eq!(tags(&mut h), json!({"a": "2", "B": ""}));
}

#[test]
fn untag_removes_listed_keys_and_ignores_unknown_ones() {
    let mut h = harness();
    tag(&mut h, json!({"a": "1", "b": "2"}));
    let r = h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["a", "zz"]}));
    assert_eq!(r, json!({"ok": true}));
    assert_eq!(tags(&mut h), json!({"b": "2"}));
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": []}));
    assert_eq!(tags(&mut h), json!({"b": "2"}));
}

#[test]
fn invalid_tags_are_rejected_without_changes() {
    let mut h = harness();
    let bad = [
        json!({"": "v"}),
        json!({"a b": "v"}),
        json!({"a.b": "v"}),
        json!({"k".repeat(129): "v"}),
        json!({"ok": "v", "n": 5}),
        json!({"ok": "v", "n": null}),
        json!({"ok": "v", "big": "x".repeat(257)}),
        json!({"ok": "v", "big": "é".repeat(129)}),
    ];
    for t in bad {
        assert_eq!(tag(&mut h, t.clone())["error"]["code"], "InvalidParameterValue", "{t}");
    }
    assert_eq!(tags(&mut h), json!({}));
    tag(&mut h, json!({"k".repeat(128): "x".repeat(256), "e": "é".repeat(128)}));
    assert_eq!(tags(&mut h).as_object().unwrap().len(), 2);
}

#[test]
fn untag_rejects_bad_keys() {
    let mut h = harness();
    tag(&mut h, json!({"a": "1"}));
    for keys in [json!(["a", 5]), json!(["a", ""]), json!(["a b"]), json!("a"), json!({})] {
        let code = h.err(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": keys}));
        assert_eq!(code, "InvalidParameterValue", "{keys}");
    }
    assert_eq!(tags(&mut h), json!({"a": "1"}));
}

#[test]
fn parameter_and_lookup_errors_follow_the_check_order() {
    let mut h = harness();
    assert_eq!(h.err(json!({"Command": "TagQueue", "QueueName": "q"})), "MissingParameter");
    assert_eq!(h.err(json!({"Command": "TagQueue", "Tags": {}})), "MissingParameter");
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "q", "Tags": []})),
        "InvalidParameterValue"
    );
    assert_eq!(h.err(json!({"Command": "UntagQueue", "QueueName": "q"})), "MissingParameter");
    assert_eq!(h.err(json!({"Command": "ListQueueTags"})), "MissingParameter");
    // Parameter values come before the queue lookup.
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "none", "Tags": {"a b": "v"}})),
        "InvalidParameterValue"
    );
    for command in [
        json!({"Command": "TagQueue", "QueueName": "none", "Tags": {}}),
        json!({"Command": "UntagQueue", "QueueName": "none", "TagKeys": []}),
        json!({"Command": "ListQueueTags", "QueueName": "none"}),
    ] {
        assert_eq!(h.err(command), "QueueDoesNotExist");
    }
}

#[test]
fn at_most_fifty_tags() {
    let mut h = harness();
    let many = |range: std::ops::Range<u32>| {
        Value::Object(range.map(|i| (format!("k{i}"), json!("v"))).collect())
    };
    tag(&mut h, many(0..50));
    // Replacing existing keys stays within the limit.
    assert_eq!(tag(&mut h, many(10..20))["ok"], true);
    let r = tag(&mut h, json!({"extra": "v"}));
    assert_eq!(r["error"]["code"], "LimitExceeded");
    // A failed command changes nothing: the mixed request adds nothing.
    let r = tag(&mut h, json!({"k0": "new", "extra": "v"}));
    assert_eq!(r["error"]["code"], "LimitExceeded");
    assert_eq!(tags(&mut h)["k0"], "v");
    assert_eq!(tags(&mut h).as_object().unwrap().len(), 50);
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["k0"]}));
    assert_eq!(tag(&mut h, json!({"extra": "v"}))["ok"], true);
    assert_eq!(tag(&mut h, many(100..151))["error"]["code"], "LimitExceeded");
}

#[test]
fn tags_are_not_attributes_and_survive_purge_but_not_delete() {
    let mut h = harness();
    tag(&mut h, json!({"a": "1"}));
    let got = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert!(got["Attributes"].get("Tags").is_none());
    // CreateQueue rule 4 ignores tags.
    assert_eq!(h.ok(json!({"Command": "CreateQueue", "QueueName": "q"})), json!({"ok": true}));
    h.ok(json!({"Command": "SendMessage", "QueueName": "q", "Body": "x"}));
    h.ok(json!({"Command": "PurgeQueue", "QueueName": "q"}));
    assert_eq!(tags(&mut h), json!({"a": "1"}));
    h.ok(json!({"Command": "DeleteQueue", "QueueName": "q"}));
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    assert_eq!(tags(&mut h), json!({}));
}
