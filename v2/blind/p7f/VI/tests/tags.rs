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
fn new_queue_has_no_tags() {
    let mut h = harness();
    assert_eq!(tags(&mut h), json!({}));
    let r = h.ok(json!({"Command": "ListQueueTags", "QueueName": "q"}));
    assert_eq!(r, json!({"ok": true, "Tags": {}}));
}

#[test]
fn tag_sets_and_replaces() {
    let mut h = harness();
    assert_eq!(tag(&mut h, json!({"a": "1", "B": ""})), json!({"ok": true}));
    assert_eq!(tags(&mut h), json!({"a": "1", "B": ""}));
    tag(&mut h, json!({"a": "2", "b": "x"}));
    assert_eq!(tags(&mut h), json!({"a": "2", "B": "", "b": "x"}));
    assert_eq!(tag(&mut h, json!({})), json!({"ok": true}));
    assert_eq!(tags(&mut h), json!({"a": "2", "B": "", "b": "x"}));
}

#[test]
fn untag_removes_and_ignores_unknown_keys() {
    let mut h = harness();
    tag(&mut h, json!({"a": "1", "b": "2"}));
    let r = h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["a", "zzz"]}));
    assert_eq!(r, json!({"ok": true}));
    assert_eq!(tags(&mut h), json!({"b": "2"}));
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": []}));
    assert_eq!(tags(&mut h), json!({"b": "2"}));
}

#[test]
fn invalid_keys_and_values() {
    let mut h = harness();
    for bad in [
        json!({"": "v"}),
        json!({"a b": "v"}),
        json!({"k\u{e9}": "v"}),
        json!({"x".repeat(129): "v"}),
        json!({"a": 1}),
        json!({"a": null}),
        json!({"a": "x".repeat(257)}),
        json!({"a": "\u{20ac}".repeat(86)}),
    ] {
        assert_eq!(
            tag(&mut h, bad.clone())["error"]["code"],
            "InvalidParameterValue",
            "{bad}"
        );
    }
    assert_eq!(tags(&mut h), json!({}));
    tag(&mut h, json!({"x".repeat(128): "x".repeat(256)}));
    tag(&mut h, json!({"e": "\u{20ac}".repeat(85)}));
    assert_eq!(tags(&mut h).as_object().unwrap().len(), 2);
}

#[test]
fn untag_rejects_bad_keys() {
    let mut h = harness();
    for bad in [json!([1]), json!(["a b"]), json!([null]), json!(["ok", ""])] {
        let code = h.err(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": bad}));
        assert_eq!(code, "InvalidParameterValue");
    }
    let code = h.err(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": "a"}));
    assert_eq!(code, "InvalidParameterValue");
    let code = h.err(json!({"Command": "TagQueue", "QueueName": "q", "Tags": ["a"]}));
    assert_eq!(code, "InvalidParameterValue");
}

#[test]
fn missing_parameters_and_unknown_queue() {
    let mut h = harness();
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
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "q", "Tags": null})),
        "MissingParameter"
    );
    for command in ["TagQueue", "UntagQueue", "ListQueueTags"] {
        let code =
            h.err(json!({"Command": command, "QueueName": "nope", "Tags": {}, "TagKeys": []}));
        assert_eq!(code, "QueueDoesNotExist", "{command}");
    }
    let code = h.err(json!({"Command": "TagQueue", "QueueName": "nope", "Tags": {"a b": "v"}}));
    assert_eq!(code, "InvalidParameterValue");
}

#[test]
fn limit_is_fifty_tags_after_the_command() {
    let mut h = harness();
    let many = |range: std::ops::Range<u32>| -> Value {
        range
            .map(|i| (format!("k{i}"), json!("v")))
            .collect::<serde_json::Map<_, _>>()
            .into()
    };
    assert_eq!(tag(&mut h, many(0..50))["ok"], true);
    assert_eq!(tag(&mut h, many(0..51))["error"]["code"], "LimitExceeded");
    assert_eq!(tags(&mut h).as_object().unwrap().len(), 50);
    assert_eq!(tag(&mut h, json!({"k0": "new"}))["ok"], true);
    assert_eq!(
        tag(&mut h, json!({"other": "v"}))["error"]["code"],
        "LimitExceeded"
    );
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["k0"]}));
    assert_eq!(tag(&mut h, json!({"other": "v"}))["ok"], true);
}

#[test]
fn failed_tag_changes_nothing() {
    let mut h = harness();
    tag(&mut h, json!({"a": "1"}));
    assert_eq!(tag(&mut h, json!({"a": "2", "b": 5}))["ok"], false);
    assert_eq!(tags(&mut h), json!({"a": "1"}));
}

#[test]
fn tags_are_not_attributes_and_survive_purge_but_not_delete() {
    let mut h = harness();
    tag(&mut h, json!({"a": "1"}));
    let r = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert!(r["Attributes"].get("Tags").is_none());
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    let code = h.err(json!({"Command": "CreateQueue", "QueueName": "q",
        "Attributes": {"Tags": {"a": "1"}}}));
    assert_eq!(code, "InvalidParameterValue");
    h.ok(json!({"Command": "PurgeQueue", "QueueName": "q"}));
    assert_eq!(tags(&mut h), json!({"a": "1"}));
    h.ok(json!({"Command": "DeleteQueue", "QueueName": "q"}));
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    assert_eq!(tags(&mut h), json!({}));
}
