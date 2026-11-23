mod common;

use common::Harness;
use serde_json::{Value, json};

fn setup() -> Harness {
    let mut h = Harness::new();
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    h
}

fn tags(h: &mut Harness) -> Value {
    h.ok(json!({"Command": "ListQueueTags", "QueueName": "q"}))["Tags"].clone()
}

fn tag(h: &mut Harness, tags: Value) -> Value {
    h.run(json!({"Command": "TagQueue", "QueueName": "q", "Tags": tags}))
}

#[test]
fn tag_replace_untag_list() {
    let mut h = setup();
    assert_eq!(tags(&mut h), json!({}));
    h.ok(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"a": "1", "b": ""}}));
    h.ok(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"a": "2", "A": "3"}}));
    assert_eq!(tags(&mut h), json!({"a": "2", "A": "3", "b": ""}));
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["a", "zzz"]}));
    assert_eq!(tags(&mut h), json!({"A": "3", "b": ""}));
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": []}));
    h.ok(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {}}));
    assert_eq!(tags(&mut h), json!({"A": "3", "b": ""}));
}

#[test]
fn parameter_errors() {
    let mut h = setup();
    let cases = [
        (
            json!({"Command": "TagQueue", "QueueName": "q"}),
            "MissingParameter",
        ),
        (
            json!({"Command": "TagQueue", "Tags": {}}),
            "MissingParameter",
        ),
        (
            json!({"Command": "TagQueue", "QueueName": "q", "Tags": null}),
            "MissingParameter",
        ),
        (
            json!({"Command": "TagQueue", "QueueName": "q", "Tags": []}),
            "InvalidParameterValue",
        ),
        (
            json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"a b": "x"}}),
            "InvalidParameterValue",
        ),
        (
            json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"": "x"}}),
            "InvalidParameterValue",
        ),
        (
            json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"a": 1}}),
            "InvalidParameterValue",
        ),
        (
            json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"a": null}}),
            "InvalidParameterValue",
        ),
        (
            json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"a": "é".repeat(129)}}),
            "InvalidParameterValue",
        ),
        (
            json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"k".repeat(129): "x"}}),
            "InvalidParameterValue",
        ),
        (
            json!({"Command": "UntagQueue", "QueueName": "q"}),
            "MissingParameter",
        ),
        (
            json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": "a"}),
            "InvalidParameterValue",
        ),
        (
            json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": [1]}),
            "InvalidParameterValue",
        ),
        (
            json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["a", "b c"]}),
            "InvalidParameterValue",
        ),
        (json!({"Command": "ListQueueTags"}), "MissingParameter"),
        // Value errors come before the queue lookup.
        (
            json!({"Command": "TagQueue", "QueueName": "nope", "Tags": {"a b": "x"}}),
            "InvalidParameterValue",
        ),
        (
            json!({"Command": "TagQueue", "QueueName": "nope", "Tags": {}}),
            "QueueDoesNotExist",
        ),
        (
            json!({"Command": "UntagQueue", "QueueName": "nope", "TagKeys": []}),
            "QueueDoesNotExist",
        ),
        (
            json!({"Command": "ListQueueTags", "QueueName": "nope"}),
            "QueueDoesNotExist",
        ),
    ];
    for (command, code) in cases {
        assert_eq!(h.err(command.clone()), code, "{command}");
    }
    assert_eq!(tags(&mut h), json!({}));
}

#[test]
fn boundary_sizes_are_accepted() {
    let mut h = setup();
    let value = "é".repeat(128);
    let key = "k".repeat(128);
    h.ok(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {key.clone(): value.clone()}}));
    assert_eq!(tags(&mut h), json!({key: value}));
}

#[test]
fn limit_is_50_tags_after_the_command() {
    let mut h = setup();
    let first: serde_json::Map<String, Value> =
        (0..50).map(|i| (format!("k{i}"), json!("v"))).collect();
    assert_eq!(tag(&mut h, Value::Object(first))["ok"], true);
    // Replacing existing keys stays within the limit.
    assert_eq!(tag(&mut h, json!({"k0": "new", "k49": "new"}))["ok"], true);
    let response = tag(&mut h, json!({"k0": "changed", "extra": "x"}));
    assert_eq!(response["error"]["code"], "LimitExceeded");
    let all = tags(&mut h);
    assert_eq!(all.as_object().unwrap().len(), 50);
    assert_eq!(all["k0"], "new");
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["k1"]}));
    assert_eq!(tag(&mut h, json!({"extra": "x"}))["ok"], true);
}

#[test]
fn tags_are_not_attributes_and_survive_purge_but_not_delete() {
    let mut h = setup();
    h.ok(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"a": "1"}}));
    // CreateQueue rule 4 ignores tags.
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    let attributes = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert!(attributes["Attributes"].get("Tags").is_none());
    h.ok(json!({"Command": "PurgeQueue", "QueueName": "q"}));
    assert_eq!(tags(&mut h), json!({"a": "1"}));
    h.ok(json!({"Command": "DeleteQueue", "QueueName": "q"}));
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    assert_eq!(tags(&mut h), json!({}));
}
