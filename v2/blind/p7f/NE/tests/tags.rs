mod common;

use common::Harness;
use serde_json::{Value, json};

fn setup() -> Harness {
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
    let mut h = setup();
    assert_eq!(tags(&mut h), json!({}));
    assert_eq!(
        h.ok(json!({"Command": "ListQueueTags", "QueueName": "q"})),
        json!({"ok": true, "Tags": {}})
    );
}

#[test]
fn tag_sets_and_replaces() {
    let mut h = setup();
    assert_eq!(tag(&mut h, json!({"a": "1", "b": ""})), json!({"ok": true}));
    tag(&mut h, json!({"a": "2", "c": "3"}));
    assert_eq!(tags(&mut h), json!({"a": "2", "b": "", "c": "3"}));
    assert_eq!(tag(&mut h, json!({})), json!({"ok": true}));
    // keys are case-sensitive
    tag(&mut h, json!({"A": "x"}));
    assert_eq!(tags(&mut h)["A"], "x");
    assert_eq!(tags(&mut h)["a"], "2");
}

#[test]
fn untag_removes_and_ignores_unknown_keys() {
    let mut h = setup();
    tag(&mut h, json!({"a": "1", "b": "2"}));
    let r = h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["a", "zzz"]}));
    assert_eq!(r, json!({"ok": true}));
    assert_eq!(tags(&mut h), json!({"b": "2"}));
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": []}));
    assert_eq!(tags(&mut h), json!({"b": "2"}));
}

#[test]
fn key_and_value_rules() {
    let mut h = setup();
    let long_key = "k".repeat(129);
    let long_value = "v".repeat(257);
    for bad in [
        json!({"bad key": "x"}),
        json!({"": "x"}),
        json!({"é": "x"}),
        json!({long_key.as_str(): "x"}),
        json!({"a": 1}),
        json!({"a": null}),
        json!({"a": long_value.as_str()}),
        // 129 two-byte characters: under 256 characters, over 256 bytes
        json!({"a": "é".repeat(129)}),
    ] {
        assert_eq!(tag(&mut h, bad.clone())["error"]["code"], "InvalidParameterValue", "{bad}");
    }
    assert_eq!(tags(&mut h), json!({}));
    let ok_key = "k".repeat(128);
    let ok_value = "é".repeat(128);
    assert_eq!(tag(&mut h, json!({ok_key.as_str(): ok_value.as_str()})), json!({"ok": true}));
}

#[test]
fn a_bad_entry_applies_nothing() {
    let mut h = setup();
    tag(&mut h, json!({"good": "1", "bad key": "2"}));
    assert_eq!(tags(&mut h), json!({}));
}

#[test]
fn at_most_fifty_tags() {
    let mut h = setup();
    let fifty: serde_json::Map<String, Value> =
        (0..50).map(|i| (format!("k{i}"), json!("v"))).collect();
    assert_eq!(tag(&mut h, Value::Object(fifty)), json!({"ok": true}));
    assert_eq!(tag(&mut h, json!({"extra": "x"}))["error"]["code"], "LimitExceeded");
    // replacing existing keys is still fine
    assert_eq!(tag(&mut h, json!({"k0": "new"})), json!({"ok": true}));
    // a failed command changes nothing, even though k1 exists
    assert_eq!(tag(&mut h, json!({"k1": "new", "extra": "x"}))["error"]["code"], "LimitExceeded");
    assert_eq!(tags(&mut h)["k1"], "v");
    // removing one makes room
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["k0"]}));
    assert_eq!(tag(&mut h, json!({"extra": "x"})), json!({"ok": true}));
    assert_eq!(tags(&mut h).as_object().unwrap().len(), 50);
}

#[test]
fn check_order() {
    let mut h = setup();
    let code = |h: &mut Harness, cmd: Value| h.err(cmd);
    assert_eq!(code(&mut h, json!({"Command": "TagQueue", "QueueName": "q"})), "MissingParameter");
    assert_eq!(code(&mut h, json!({"Command": "TagQueue", "Tags": {}})), "MissingParameter");
    assert_eq!(code(&mut h, json!({"Command": "TagQueue", "QueueName": "q", "Tags": []})), "InvalidParameterValue");
    // value errors come before the queue lookup
    assert_eq!(code(&mut h, json!({"Command": "TagQueue", "QueueName": "nope", "Tags": {"a": 1}})), "InvalidParameterValue");
    assert_eq!(code(&mut h, json!({"Command": "TagQueue", "QueueName": "nope", "Tags": {}})), "QueueDoesNotExist");
    assert_eq!(code(&mut h, json!({"Command": "UntagQueue", "QueueName": "q"})), "MissingParameter");
    for keys in [json!({}), json!([1]), json!(["bad key"]), json!([""])] {
        assert_eq!(code(&mut h, json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": keys})), "InvalidParameterValue");
    }
    assert_eq!(code(&mut h, json!({"Command": "UntagQueue", "QueueName": "nope", "TagKeys": []})), "QueueDoesNotExist");
    assert_eq!(code(&mut h, json!({"Command": "ListQueueTags"})), "MissingParameter");
    assert_eq!(code(&mut h, json!({"Command": "ListQueueTags", "QueueName": 3})), "InvalidParameterValue");
    assert_eq!(code(&mut h, json!({"Command": "ListQueueTags", "QueueName": "nope"})), "QueueDoesNotExist");
}

#[test]
fn tags_are_not_attributes_and_survive_purge_not_delete() {
    let mut h = setup();
    tag(&mut h, json!({"a": "1"}));
    let r = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(r["Attributes"].get("Tags"), None);
    // CreateQueue rule 4 ignores tags
    assert_eq!(
        h.run(json!({"Command": "CreateQueue", "QueueName": "q"})),
        json!({"ok": true})
    );
    assert_eq!(tags(&mut h), json!({"a": "1"}));
    h.ok(json!({"Command": "PurgeQueue", "QueueName": "q"}));
    assert_eq!(tags(&mut h), json!({"a": "1"}));
    h.ok(json!({"Command": "DeleteQueue", "QueueName": "q"}));
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    assert_eq!(tags(&mut h), json!({}));
}
