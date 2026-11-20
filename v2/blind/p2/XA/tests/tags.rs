mod common;

use common::Harness;
use serde_json::{Value, json};

fn queue() -> Harness {
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

fn many(range: std::ops::Range<u32>) -> Value {
    range
        .map(|i| (format!("k{i}"), json!("v")))
        .collect::<serde_json::Map<_, _>>()
        .into()
}

#[test]
fn tags_start_empty_and_are_set_replaced_and_removed() {
    let mut h = queue();
    assert_eq!(list(&mut h), json!({}));
    assert_eq!(
        h.ok(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"a": "1", "B": ""}})),
        json!({"ok": true})
    );
    assert_eq!(list(&mut h), json!({"a": "1", "B": ""}));
    // Keys are case-sensitive; a given key replaces the old value.
    h.ok(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"a": "2", "b": "3"}}));
    assert_eq!(list(&mut h), json!({"a": "2", "B": "", "b": "3"}));
    // Unknown keys are ignored.
    assert_eq!(
        h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["a", "zzz"]})),
        json!({"ok": true})
    );
    assert_eq!(list(&mut h), json!({"B": "", "b": "3"}));
    h.ok(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {}}));
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": []}));
    assert_eq!(list(&mut h), json!({"B": "", "b": "3"}));
}

#[test]
fn key_and_value_rules() {
    let mut h = queue();
    for bad in [
        json!({"": "v"}),
        json!({"a b": "v"}),
        json!({"é": "v"}),
        json!({"k".repeat(129): "v"}),
        json!({"k": 1}),
        json!({"k": null}),
        json!({"k": "v".repeat(257)}),
        json!({"k": "é".repeat(129)}), // 258 bytes
    ] {
        assert_eq!(
            tag(&mut h, bad.clone())["error"]["code"],
            "InvalidParameterValue",
            "{bad}"
        );
    }
    assert_eq!(list(&mut h), json!({}));
    assert_eq!(
        tag(&mut h, json!({"k".repeat(128): "v".repeat(256)}))["ok"],
        true
    );
    assert_eq!(tag(&mut h, json!({"k": "é".repeat(128)}))["ok"], true); // 256 bytes
    for bad in [json!([1]), json!(["a b"]), json!([""]), json!([null])] {
        assert_eq!(
            h.err(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": bad})),
            "InvalidParameterValue"
        );
    }
}

#[test]
fn parameter_check_order() {
    let mut h = queue();
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
        h.err(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": {}})),
        "InvalidParameterValue"
    );
    // Values are checked before the queue lookup.
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "nope", "Tags": {"a b": "v"}})),
        "InvalidParameterValue"
    );
    for command in [
        json!({"Command": "TagQueue", "QueueName": "nope", "Tags": {}}),
        json!({"Command": "UntagQueue", "QueueName": "nope", "TagKeys": []}),
        json!({"Command": "ListQueueTags", "QueueName": "nope"}),
    ] {
        assert_eq!(h.err(command), "QueueDoesNotExist");
    }
}

#[test]
fn at_most_fifty_tags() {
    let mut h = queue();
    assert_eq!(tag(&mut h, many(0..50))["ok"], true);
    // A new key is over the limit and nothing of the command is applied.
    let r = tag(&mut h, json!({"k0": "changed", "extra": "x"}));
    assert_eq!(r["error"]["code"], "LimitExceeded");
    assert_eq!(list(&mut h)["k0"], "v");
    assert_eq!(list(&mut h).as_object().unwrap().len(), 50);
    // Replacing existing keys is fine at the limit.
    assert_eq!(tag(&mut h, json!({"k0": "changed"}))["ok"], true);
    assert_eq!(list(&mut h)["k0"], "changed");
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["k1"]}));
    assert_eq!(tag(&mut h, json!({"extra": "x"}))["ok"], true);
    // 51 new keys at once fail, too.
    let mut h = queue();
    assert_eq!(tag(&mut h, many(0..51))["error"]["code"], "LimitExceeded");
    assert_eq!(list(&mut h), json!({}));
}

#[test]
fn tags_are_not_attributes_and_survive_purge_but_not_delete() {
    let mut h = queue();
    tag(&mut h, json!({"a": "1"}));
    let attrs =
        h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}))["Attributes"].clone();
    assert_eq!(
        attrs,
        json!({
            "VisibilityTimeout": 30, "DelaySeconds": 0, "RetentionSeconds": 345600,
            "Fifo": false, "ContentBasedDeduplication": false, "MaxMessageBytes": 1024,
        })
    );
    // CreateQueue rule 4 ignores tags.
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    h.ok(json!({"Command": "PurgeQueue", "QueueName": "q"}));
    assert_eq!(list(&mut h), json!({"a": "1"}));
    h.ok(json!({"Command": "DeleteQueue", "QueueName": "q"}));
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    assert_eq!(list(&mut h), json!({}));
}
