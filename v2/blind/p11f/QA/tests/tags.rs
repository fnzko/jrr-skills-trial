mod common;

use common::Harness;
use serde_json::{Value, json};

fn harness_with_queue() -> Harness {
    let mut h = Harness::new();
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    h
}

fn tag(h: &mut Harness, tags: Value) -> Value {
    h.run(json!({"Command": "TagQueue", "QueueName": "q", "Tags": tags}))
}

fn list(h: &mut Harness) -> Value {
    h.ok(json!({"Command": "ListQueueTags", "QueueName": "q"}))
}

#[test]
fn new_queue_has_no_tags() {
    let mut h = harness_with_queue();
    assert_eq!(list(&mut h), json!({"ok": true, "Tags": {}}));
}

#[test]
fn tag_sets_and_replaces_values_and_untag_ignores_missing_keys() {
    let mut h = harness_with_queue();
    assert_eq!(tag(&mut h, json!({"a": "1", "b": ""})), json!({"ok": true}));
    assert_eq!(
        tag(&mut h, json!({"a": "2", "C": "3"})),
        json!({"ok": true})
    );
    assert_eq!(list(&mut h)["Tags"], json!({"a": "2", "b": "", "C": "3"}));
    let r = h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["a", "nope", "c"]}));
    assert_eq!(r, json!({"ok": true}));
    assert_eq!(list(&mut h)["Tags"], json!({"b": "", "C": "3"}));
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": []}));
    assert_eq!(list(&mut h)["Tags"], json!({"b": "", "C": "3"}));
}

#[test]
fn empty_tags_object_is_accepted() {
    let mut h = harness_with_queue();
    assert_eq!(tag(&mut h, json!({})), json!({"ok": true}));
}

#[test]
fn invalid_keys_and_values_are_invalid_parameter_values() {
    let mut h = harness_with_queue();
    let long_value = "v".repeat(257);
    let long_key = "k".repeat(129);
    let bad_tags = [
        json!({"": "v"}),
        json!({"a b": "v"}),
        json!({long_key: "v"}),
        json!({"a": 1}),
        json!({"a": null}),
        json!({"a": long_value}),
        json!([]),
        json!("x"),
    ];
    for bad in bad_tags {
        assert_eq!(
            tag(&mut h, bad.clone())["error"]["code"],
            "InvalidParameterValue",
            "{bad}"
        );
    }
    assert_eq!(list(&mut h)["Tags"], json!({}));
}

#[test]
fn value_limit_counts_utf8_bytes() {
    let mut h = harness_with_queue();
    // 128 two-byte characters are exactly 256 bytes.
    assert_eq!(
        tag(&mut h, json!({"a": "é".repeat(128)})),
        json!({"ok": true})
    );
    assert_eq!(
        tag(&mut h, json!({"a": "é".repeat(129)}))["error"]["code"],
        "InvalidParameterValue"
    );
    assert_eq!(
        tag(&mut h, json!({"a": "v".repeat(256)})),
        json!({"ok": true})
    );
}

#[test]
fn untag_rejects_bad_key_elements() {
    let mut h = harness_with_queue();
    for bad in [
        json!([1]),
        json!(["a b"]),
        json!([""]),
        json!({}),
        json!("a"),
    ] {
        let code = h.err(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": bad}));
        assert_eq!(code, "InvalidParameterValue", "{bad}");
    }
}

#[test]
fn fifty_tags_fit_and_a_fifty_first_does_nothing() {
    let mut h = harness_with_queue();
    let fifty: serde_json::Map<String, Value> =
        (0..50).map(|i| (format!("k{i}"), json!("v"))).collect();
    assert_eq!(tag(&mut h, Value::Object(fifty)), json!({"ok": true}));
    // A mix of a replacement and a new key must fail as a whole.
    assert_eq!(
        tag(&mut h, json!({"k0": "changed", "extra": "v"}))["error"]["code"],
        "LimitExceeded"
    );
    let tags = list(&mut h)["Tags"].clone();
    assert_eq!(tags.as_object().unwrap().len(), 50);
    assert_eq!(tags["k0"], "v");
    assert_eq!(tag(&mut h, json!({"k0": "changed"})), json!({"ok": true}));
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["k1"]}));
    assert_eq!(tag(&mut h, json!({"extra": "v"})), json!({"ok": true}));
}

#[test]
fn missing_parameters_and_unknown_queues() {
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
        h.err(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": null})),
        "MissingParameter"
    );
    assert_eq!(
        h.err(json!({"Command": "ListQueueTags"})),
        "MissingParameter"
    );
    for cmd in [
        json!({"Command": "TagQueue", "QueueName": "x", "Tags": {}}),
        json!({"Command": "UntagQueue", "QueueName": "x", "TagKeys": []}),
        json!({"Command": "ListQueueTags", "QueueName": "x"}),
    ] {
        assert_eq!(h.err(cmd), "QueueDoesNotExist");
    }
}

#[test]
fn value_errors_come_before_the_queue_lookup() {
    let mut h = Harness::new();
    let code = h.err(json!({"Command": "TagQueue", "QueueName": "x", "Tags": {"a": 1}}));
    assert_eq!(code, "InvalidParameterValue");
}

#[test]
fn tags_are_not_attributes_and_survive_purge_but_not_delete() {
    let mut h = harness_with_queue();
    tag(&mut h, json!({"a": "1"}));
    let attrs = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(attrs["Attributes"].get("Tags"), None);
    h.ok(json!({"Command": "SendMessage", "QueueName": "q", "Body": "x"}));
    h.ok(json!({"Command": "PurgeQueue", "QueueName": "q"}));
    assert_eq!(list(&mut h)["Tags"], json!({"a": "1"}));
    // Re-creating with the same attributes is still a no-op despite the tags.
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    assert_eq!(list(&mut h)["Tags"], json!({"a": "1"}));
    h.ok(json!({"Command": "DeleteQueue", "QueueName": "q"}));
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    assert_eq!(list(&mut h)["Tags"], json!({}));
}

#[test]
fn tags_of_different_queues_are_independent() {
    let mut h = harness_with_queue();
    h.ok(json!({"Command": "CreateQueue", "QueueName": "other"}));
    tag(&mut h, json!({"a": "1"}));
    let r = h.ok(json!({"Command": "ListQueueTags", "QueueName": "other"}));
    assert_eq!(r["Tags"], json!({}));
}
