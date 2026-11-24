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

fn list(h: &mut Harness) -> Value {
    h.ok(json!({"Command": "ListQueueTags", "QueueName": "q"}))["Tags"].clone()
}

fn code(response: &Value) -> &str {
    response["error"]["code"]
        .as_str()
        .expect("a failure response")
}

#[test]
fn new_queue_has_no_tags() {
    assert_eq!(list(&mut harness()), json!({}));
}

#[test]
fn tag_sets_and_replaces_values() {
    let mut h = harness();
    assert_eq!(tag(&mut h, json!({"a": "1", "b": ""})), json!({"ok": true}));
    tag(&mut h, json!({"a": "2"}));
    assert_eq!(list(&mut h), json!({"a": "2", "b": ""}));
}

#[test]
fn untag_removes_listed_keys_and_ignores_unknown() {
    let mut h = harness();
    tag(&mut h, json!({"a": "1", "b": "2"}));
    let r = h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["a", "zzz"]}));
    assert_eq!(r, json!({"ok": true}));
    assert_eq!(list(&mut h), json!({"b": "2"}));
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": []}));
    assert_eq!(list(&mut h), json!({"b": "2"}));
}

#[test]
fn keys_are_case_sensitive() {
    let mut h = harness();
    tag(&mut h, json!({"a": "1", "A": "2"}));
    assert_eq!(list(&mut h), json!({"a": "1", "A": "2"}));
}

#[test]
fn invalid_keys_and_values_change_nothing() {
    let mut h = harness();
    let long_key = "k".repeat(129);
    let long_value = "v".repeat(257);
    for bad in [
        json!({"ok": "1", "bad key": "1"}),
        json!({"": "1"}),
        json!({long_key: "1"}),
        json!({"ok": 1}),
        json!({"ok": null}),
        json!({"ok": long_value}),
    ] {
        assert_eq!(
            code(&tag(&mut h, bad.clone())),
            "InvalidParameterValue",
            "{bad}"
        );
    }
    assert_eq!(list(&mut h), json!({}));
    assert_eq!(
        tag(&mut h, json!({"k": "v".repeat(256), "é": "x"}))["error"]["code"],
        "InvalidParameterValue"
    );
    assert_eq!(tag(&mut h, json!({"k": "é".repeat(128)}))["ok"], true);
    assert_eq!(
        tag(&mut h, json!({"k2": "é".repeat(129)}))["error"]["code"],
        "InvalidParameterValue"
    );
}

#[test]
fn parameter_errors_follow_the_check_order() {
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
        h.err(json!({"Command": "TagQueue", "QueueName": "q", "Tags": []})),
        "InvalidParameterValue"
    );
    // Value errors come before the queue lookup.
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "nope", "Tags": {"a": 1}})),
        "InvalidParameterValue"
    );
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "nope", "Tags": {}})),
        "QueueDoesNotExist"
    );
    assert_eq!(
        h.err(json!({"Command": "UntagQueue", "QueueName": "q"})),
        "MissingParameter"
    );
    for bad in [json!("a"), json!([1]), json!(["a b"]), json!([""])] {
        assert_eq!(
            h.err(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": bad})),
            "InvalidParameterValue"
        );
    }
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

fn numbered(range: std::ops::Range<u32>) -> Value {
    Value::Object(range.map(|i| (format!("k{i}"), json!("v"))).collect())
}

#[test]
fn fifty_tags_fit_and_fifty_one_do_not() {
    let mut h = harness();
    assert_eq!(tag(&mut h, numbered(0..50))["ok"], true);
    assert_eq!(code(&tag(&mut h, json!({"extra": "v"}))), "LimitExceeded");
    // Replacing an existing key adds no tag.
    assert_eq!(tag(&mut h, json!({"k0": "new"}))["ok"], true);
    assert_eq!(list(&mut h).as_object().map(|o| o.len()), Some(50));
    assert_eq!(list(&mut h)["k0"], "new");
}

#[test]
fn over_limit_batch_sets_none_of_its_tags() {
    let mut h = harness();
    tag(&mut h, numbered(0..49));
    assert_eq!(
        code(&tag(&mut h, json!({"k0": "new", "x": "1", "y": "2"}))),
        "LimitExceeded"
    );
    assert_eq!(list(&mut h)["k0"], "v");
    assert_eq!(list(&mut h).as_object().map(|o| o.len()), Some(49));
}

#[test]
fn tags_are_not_attributes_and_survive_purge_but_not_delete() {
    let mut h = harness();
    tag(&mut h, json!({"a": "1"}));
    let attrs = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(attrs["Attributes"].get("a"), None);
    assert_eq!(attrs["Attributes"].get("Tags"), None);
    assert_eq!(attrs.get("Tags"), None);
    // CreateQueue rule 4 ignores tags.
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    h.ok(json!({"Command": "PurgeQueue", "QueueName": "q"}));
    assert_eq!(list(&mut h), json!({"a": "1"}));
    h.ok(json!({"Command": "DeleteQueue", "QueueName": "q"}));
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    assert_eq!(list(&mut h), json!({}));
}
