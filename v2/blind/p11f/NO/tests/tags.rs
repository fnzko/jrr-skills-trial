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
fn tags_start_empty_and_are_set_and_replaced() {
    let mut h = harness();
    assert_eq!(list(&mut h), json!({"ok": true, "Tags": {}}));
    assert_eq!(tag(&mut h, json!({"a": "1", "B": ""})), json!({"ok": true}));
    assert_eq!(list(&mut h)["Tags"], json!({"a": "1", "B": ""}));
    tag(&mut h, json!({"a": "2", "b": "3"}));
    assert_eq!(list(&mut h)["Tags"], json!({"a": "2", "B": "", "b": "3"}));
    assert_eq!(tag(&mut h, json!({})), json!({"ok": true}));
}

#[test]
fn untag_removes_and_ignores_missing_keys() {
    let mut h = harness();
    tag(&mut h, json!({"a": "1", "b": "2"}));
    let r = h.run(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["a", "zz"]}));
    assert_eq!(r, json!({"ok": true}));
    assert_eq!(list(&mut h)["Tags"], json!({"b": "2"}));
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": []}));
}

#[test]
fn invalid_tag_input_is_a_parameter_error_before_the_lookup() {
    let mut h = Harness::new();
    let long_value = "x".repeat(257);
    let long_key = "k".repeat(129);
    for tags in [
        json!({"bad key": "v"}),
        json!({"": "v"}),
        json!({long_key: "v"}),
        json!({"k": 1}),
        json!({"k": null}),
        json!({"k": long_value}),
        json!([]),
    ] {
        let command = json!({"Command": "TagQueue", "QueueName": "none", "Tags": tags});
        assert_eq!(h.err(command), "InvalidParameterValue");
    }
    for keys in [json!(["a", 1]), json!(["a b"]), json!({}), json!([""])] {
        let command = json!({"Command": "UntagQueue", "QueueName": "none", "TagKeys": keys});
        assert_eq!(h.err(command), "InvalidParameterValue");
    }
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "none"})),
        "MissingParameter"
    );
    assert_eq!(
        h.err(json!({"Command": "UntagQueue", "QueueName": "none"})),
        "MissingParameter"
    );
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "none", "Tags": {}})),
        "QueueDoesNotExist"
    );
    assert_eq!(
        h.err(json!({"Command": "ListQueueTags", "QueueName": "none"})),
        "QueueDoesNotExist"
    );
}

#[test]
fn value_limit_counts_utf8_bytes() {
    let mut h = harness();
    assert_eq!(
        tag(&mut h, json!({"k": "\u{e9}".repeat(128)})),
        json!({"ok": true})
    );
    assert_eq!(
        tag(&mut h, json!({"k": "\u{e9}".repeat(129)}))["error"]["code"],
        "InvalidParameterValue"
    );
}

#[test]
fn at_most_fifty_tags_and_a_failed_command_changes_nothing() {
    let mut h = harness();
    let many = |range: std::ops::Range<u32>| {
        Value::Object(range.map(|i| (format!("k{i}"), json!("v"))).collect())
    };
    tag(&mut h, many(0..50));
    // Replacing existing keys stays within the limit.
    assert_eq!(tag(&mut h, json!({"k0": "new"})), json!({"ok": true}));
    assert_eq!(
        tag(&mut h, json!({"fresh": "v", "k0": "newer"}))["error"]["code"],
        "LimitExceeded"
    );
    let tags = list(&mut h)["Tags"].clone();
    assert_eq!(tags.as_object().unwrap().len(), 50);
    assert_eq!(tags["k0"], "new");
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["k1"]}));
    assert_eq!(tag(&mut h, json!({"fresh": "v"})), json!({"ok": true}));
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
