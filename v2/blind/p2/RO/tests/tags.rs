mod common;

use common::Harness;
use serde_json::{Value, json};

fn setup() -> Harness {
    let mut h = Harness::new();
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    h
}

fn tag(h: &mut Harness, tags: Value) {
    h.ok(json!({"Command": "TagQueue", "QueueName": "q", "Tags": tags}));
}

fn tags(h: &mut Harness) -> Value {
    h.ok(json!({"Command": "ListQueueTags", "QueueName": "q"}))["Tags"].clone()
}

#[test]
fn new_queue_has_no_tags_and_tagging_replaces_values() {
    let mut h = setup();
    assert_eq!(tags(&mut h), json!({}));
    tag(&mut h, json!({"a": "1", "B": ""}));
    tag(&mut h, json!({"a": "2"}));
    assert_eq!(tags(&mut h), json!({"a": "2", "B": ""}));
    tag(&mut h, json!({}));
    assert_eq!(tags(&mut h), json!({"a": "2", "B": ""}));
}

#[test]
fn untag_removes_and_ignores_unknown_keys() {
    let mut h = setup();
    tag(&mut h, json!({"a": "1", "b": "2"}));
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["a", "zzz"]}));
    assert_eq!(tags(&mut h), json!({"b": "2"}));
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": []}));
    assert_eq!(tags(&mut h), json!({"b": "2"}));
}

#[test]
fn key_and_value_rules() {
    let mut h = setup();
    let bad = [
        json!({"": "v"}),
        json!({"a b": "v"}),
        json!({"é": "v"}),
        json!({"k".repeat(129): "v"}),
        json!({"a": 1}),
        json!({"a": null}),
        json!({"a": "x".repeat(257)}),
        json!({"a": "é".repeat(129)}),
    ];
    for tags in bad {
        assert_eq!(
            h.err(json!({"Command": "TagQueue", "QueueName": "q", "Tags": tags})),
            "InvalidParameterValue",
            "{tags}"
        );
    }
    tag(&mut h, json!({"k".repeat(128): "x".repeat(256)}));
    tag(&mut h, json!({"b": "é".repeat(128)}));
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "q", "Tags": []})),
        "InvalidParameterValue"
    );
    for keys in [json!([1]), json!(["a b"]), json!("a")] {
        assert_eq!(
            h.err(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": keys})),
            "InvalidParameterValue"
        );
    }
}

#[test]
fn check_order() {
    let mut h = setup();
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
    // Value errors come before the lookup, the lookup before the limit.
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "nope", "Tags": {"a": 1}})),
        "InvalidParameterValue"
    );
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "nope", "Tags": {}})),
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
fn at_most_fifty_tags_counting_replacements_as_free() {
    let mut h = setup();
    let fifty: serde_json::Map<String, Value> =
        (0..50).map(|i| (format!("k{i}"), json!("v"))).collect();
    tag(&mut h, Value::Object(fifty));
    // Replacing existing keys stays at 50.
    tag(&mut h, json!({"k0": "new", "k49": "new"}));
    // One new key would make 51; the command changes nothing, even for its other keys.
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"k1": "changed", "extra": "x"}})),
        "LimitExceeded"
    );
    let now = tags(&mut h);
    assert_eq!(now.as_object().unwrap().len(), 50);
    assert_eq!(now["k1"], "v");
    assert_eq!(now["k0"], "new");
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["k1"]}));
    tag(&mut h, json!({"extra": "x"}));
}

#[test]
fn tags_are_not_attributes_and_survive_purge_but_not_delete() {
    let mut h = setup();
    tag(&mut h, json!({"a": "1"}));
    // CreateQueue rule 4 ignores tags.
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    let attrs = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert!(attrs["Attributes"].get("Tags").is_none());
    assert!(attrs.get("Tags").is_none());
    h.ok(json!({"Command": "PurgeQueue", "QueueName": "q"}));
    assert_eq!(tags(&mut h), json!({"a": "1"}));
    h.ok(json!({"Command": "DeleteQueue", "QueueName": "q"}));
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    assert_eq!(tags(&mut h), json!({}));
}
