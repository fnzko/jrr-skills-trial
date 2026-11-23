mod common;

use common::Harness;
use serde_json::{Value, json};

fn create(h: &mut Harness, name: &str) {
    h.ok(json!({"Command": "CreateQueue", "QueueName": name}));
}

fn tag(h: &mut Harness, queue: &str, tags: Value) -> Value {
    h.run(json!({"Command": "TagQueue", "QueueName": queue, "Tags": tags}))
}

fn tags(h: &mut Harness, queue: &str) -> Value {
    h.ok(json!({"Command": "ListQueueTags", "QueueName": queue}))["Tags"].clone()
}

#[test]
fn new_queue_has_no_tags() {
    let mut h = Harness::new();
    create(&mut h, "q");
    assert_eq!(
        h.ok(json!({"Command": "ListQueueTags", "QueueName": "q"})),
        json!({"ok": true, "Tags": {}})
    );
}

#[test]
fn tag_sets_and_replaces_values() {
    let mut h = Harness::new();
    create(&mut h, "q");
    assert_eq!(
        tag(&mut h, "q", json!({"a": "1", "B": ""})),
        json!({"ok": true})
    );
    assert_eq!(tags(&mut h, "q"), json!({"a": "1", "B": ""}));
    tag(&mut h, "q", json!({"a": "2", "c": "3"}));
    assert_eq!(tags(&mut h, "q"), json!({"a": "2", "B": "", "c": "3"}));
    tag(&mut h, "q", json!({}));
    assert_eq!(tags(&mut h, "q"), json!({"a": "2", "B": "", "c": "3"}));
}

#[test]
fn keys_are_case_sensitive_and_per_queue() {
    let mut h = Harness::new();
    create(&mut h, "q");
    create(&mut h, "r");
    tag(&mut h, "q", json!({"k": "x", "K": "y"}));
    assert_eq!(tags(&mut h, "q"), json!({"k": "x", "K": "y"}));
    assert_eq!(tags(&mut h, "r"), json!({}));
}

#[test]
fn invalid_keys_and_values_are_rejected_without_effect() {
    let mut h = Harness::new();
    create(&mut h, "q");
    let long_key = "k".repeat(129);
    let long_value = "v".repeat(257);
    let cases = [
        json!({"": "v"}),
        json!({"a b": "v"}),
        json!({"é": "v"}),
        json!({long_key: "v"}),
        json!({"good": 1}),
        json!({"good": null}),
        json!({"good": long_value}),
        // The valid member does not survive next to an invalid one.
        json!({"ok": "v", "bad key": "v"}),
    ];
    for bad in cases {
        let response = tag(&mut h, "q", bad.clone());
        assert_eq!(response["error"]["code"], "InvalidParameterValue", "{bad}");
    }
    assert_eq!(tags(&mut h, "q"), json!({}));
}

#[test]
fn value_and_key_limits_are_inclusive_and_counted_correctly() {
    let mut h = Harness::new();
    create(&mut h, "q");
    let key = "k".repeat(128);
    // 128 two-byte characters are 256 bytes.
    let value = "é".repeat(128);
    assert_eq!(
        tag(&mut h, "q", json!({key.clone(): value.clone()})),
        json!({"ok": true})
    );
    assert_eq!(tags(&mut h, "q")[&key], json!(value));
    let too_long = "é".repeat(128) + "a";
    assert_eq!(
        tag(&mut h, "q", json!({"x": too_long}))["error"]["code"],
        "InvalidParameterValue"
    );
}

#[test]
fn at_most_50_tags() {
    let mut h = Harness::new();
    create(&mut h, "q");
    let fifty: serde_json::Map<String, Value> =
        (0..50).map(|i| (format!("k{i}"), json!("v"))).collect();
    assert_eq!(tag(&mut h, "q", Value::Object(fifty)), json!({"ok": true}));
    let response = tag(&mut h, "q", json!({"extra": "v"}));
    assert_eq!(response["error"]["code"], "LimitExceeded");
    // Replacing an existing key does not add a tag.
    assert_eq!(tag(&mut h, "q", json!({"k0": "new"})), json!({"ok": true}));
    // A failed command changes nothing, even for the members that would fit.
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["k0"]}));
    let response = tag(&mut h, "q", json!({"k0": "x", "e1": "y"}));
    assert_eq!(response["error"]["code"], "LimitExceeded");
    assert_eq!(tags(&mut h, "q").as_object().unwrap().len(), 49);
    assert_eq!(tags(&mut h, "q").get("k0"), None);
}

#[test]
fn a_single_command_cannot_carry_more_than_50_tags() {
    let mut h = Harness::new();
    create(&mut h, "q");
    let many: serde_json::Map<String, Value> =
        (0..51).map(|i| (format!("k{i}"), json!("v"))).collect();
    assert_eq!(
        tag(&mut h, "q", Value::Object(many))["error"]["code"],
        "LimitExceeded"
    );
}

#[test]
fn untag_removes_keys_and_ignores_unknown_ones() {
    let mut h = Harness::new();
    create(&mut h, "q");
    tag(&mut h, "q", json!({"a": "1", "b": "2", "c": "3"}));
    let r = h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["a", "zzz", "a"]}));
    assert_eq!(r, json!({"ok": true}));
    assert_eq!(tags(&mut h, "q"), json!({"b": "2", "c": "3"}));
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": []}));
    assert_eq!(tags(&mut h, "q"), json!({"b": "2", "c": "3"}));
}

#[test]
fn untag_rejects_bad_keys_without_effect() {
    let mut h = Harness::new();
    create(&mut h, "q");
    tag(&mut h, "q", json!({"a": "1"}));
    for bad in [
        json!(["a", 5]),
        json!(["a", "bad key"]),
        json!(["a", ""]),
        json!("a"),
        json!({}),
    ] {
        let command = json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": bad});
        assert_eq!(h.err(command), "InvalidParameterValue", "{bad}");
    }
    assert_eq!(tags(&mut h, "q"), json!({"a": "1"}));
}

#[test]
fn check_order_for_tag_commands() {
    let mut h = Harness::new();
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
        h.err(json!({"Command": "TagQueue", "QueueName": "q", "Tags": []})),
        "InvalidParameterValue"
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
    assert_eq!(
        h.err(json!({"Command": "ListQueueTags", "QueueName": 5})),
        "InvalidParameterValue"
    );
}

#[test]
fn tags_are_not_attributes_and_survive_purge_but_not_delete() {
    let mut h = Harness::new();
    create(&mut h, "q");
    tag(&mut h, "q", json!({"a": "1"}));
    let attributes = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(attributes["Attributes"].get("Tags"), None);
    // CreateQueue rule 4 ignores tags: a repeated create still succeeds.
    create(&mut h, "q");
    h.ok(json!({"Command": "SendMessage", "QueueName": "q", "Body": "x"}));
    h.ok(json!({"Command": "PurgeQueue", "QueueName": "q"}));
    assert_eq!(tags(&mut h, "q"), json!({"a": "1"}));
    h.ok(json!({"Command": "DeleteQueue", "QueueName": "q"}));
    create(&mut h, "q");
    assert_eq!(tags(&mut h, "q"), json!({}));
}
