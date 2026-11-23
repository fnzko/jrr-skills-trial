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
    h.ok(json!({"Command": "ListQueueTags", "QueueName": "q"}))
}

#[test]
fn new_queue_has_no_tags_and_responses_have_exact_shape() {
    let mut h = harness();
    assert_eq!(list(&mut h), json!({"ok": true, "Tags": {}}));
    assert_eq!(tag(&mut h, json!({})), json!({"ok": true}));
    assert_eq!(
        h.run(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": []})),
        json!({"ok": true})
    );
}

#[test]
fn tag_sets_replaces_and_untag_removes() {
    let mut h = harness();
    tag(&mut h, json!({"a": "1", "b": ""}));
    tag(&mut h, json!({"a": "2"}));
    assert_eq!(list(&mut h)["Tags"], json!({"a": "2", "b": ""}));
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["a", "zzz"]}));
    assert_eq!(list(&mut h)["Tags"], json!({"b": ""}));
}

#[test]
fn keys_are_case_sensitive() {
    let mut h = harness();
    tag(&mut h, json!({"k": "1", "K": "2"}));
    assert_eq!(list(&mut h)["Tags"], json!({"k": "1", "K": "2"}));
}

#[test]
fn invalid_tag_input_is_invalid_parameter_value() {
    let mut h = harness();
    let long_key = "k".repeat(129);
    let long_value = "v".repeat(257);
    for tags in [
        json!({"": "v"}),
        json!({"a b": "v"}),
        json!({long_key: "v"}),
        json!({"k": 1}),
        json!({"k": null}),
        json!({"k": long_value}),
        json!("x"),
    ] {
        assert_eq!(
            h.err(json!({"Command": "TagQueue", "QueueName": "q", "Tags": tags})),
            "InvalidParameterValue",
            "{tags}"
        );
    }
    for keys in [json!(["é"]), json!([1]), json!([""]), json!({}), json!("a")] {
        assert_eq!(
            h.err(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": keys})),
            "InvalidParameterValue",
            "{keys}"
        );
    }
    assert_eq!(list(&mut h)["Tags"], json!({}));
}

#[test]
fn value_limit_counts_bytes_and_key_limit_counts_characters() {
    let mut h = harness();
    // 128 two-byte characters are 256 bytes.
    assert_eq!(
        tag(&mut h, json!({"k".repeat(128): "é".repeat(128)}))["ok"],
        true
    );
    assert_eq!(
        tag(&mut h, json!({"j": "é".repeat(128) + "a"}))["ok"],
        false
    );
}

#[test]
fn missing_parameters_and_unknown_queue() {
    let mut h = harness();
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "q"})),
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
    assert_eq!(
        h.err(json!({"Command": "ListQueueTags", "QueueName": "nope"})),
        "QueueDoesNotExist"
    );
    // Value checks (step 3) come before the lookup (step 4).
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "nope", "Tags": {"k": 1}})),
        "InvalidParameterValue"
    );
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "nope", "Tags": {}})),
        "QueueDoesNotExist"
    );
}

#[test]
fn at_most_fifty_tags() {
    let mut h = harness();
    let fifty: serde_json::Map<String, Value> =
        (0..50).map(|i| (format!("k{i}"), json!("v"))).collect();
    assert_eq!(tag(&mut h, Value::Object(fifty))["ok"], true);
    assert_eq!(tag(&mut h, json!({"k0": "new", "k1": "new"}))["ok"], true);
    let r = tag(&mut h, json!({"extra": "v", "k0": "changed"}));
    assert_eq!(r["error"]["code"], "LimitExceeded");
    let tags = list(&mut h)["Tags"].clone();
    assert_eq!(tags.as_object().unwrap().len(), 50);
    assert_eq!(tags["k0"], "new");
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["k5"]}));
    assert_eq!(tag(&mut h, json!({"extra": "v"}))["ok"], true);
}

#[test]
fn tags_are_not_attributes_and_survive_purge_but_not_delete() {
    let mut h = harness();
    tag(&mut h, json!({"a": "1"}));
    let attrs = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert!(attrs["Attributes"].get("Tags").is_none());
    // CreateQueue rule 4 ignores tags.
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    h.ok(json!({"Command": "PurgeQueue", "QueueName": "q"}));
    assert_eq!(list(&mut h)["Tags"], json!({"a": "1"}));
    h.ok(json!({"Command": "DeleteQueue", "QueueName": "q"}));
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    assert_eq!(list(&mut h)["Tags"], json!({}));
}
