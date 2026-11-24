mod common;

use common::Harness;
use serde_json::{Value, json};

fn harness_with_queue() -> Harness {
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
fn new_queue_has_no_tags() {
    let mut h = harness_with_queue();
    assert_eq!(tags(&mut h), json!({}));
}

#[test]
fn tag_sets_and_replaces_values() {
    let mut h = harness_with_queue();
    tag(&mut h, json!({"env": "dev", "team": ""}));
    tag(&mut h, json!({"env": "prod", "Env": "case"}));
    assert_eq!(
        tags(&mut h),
        json!({"env": "prod", "team": "", "Env": "case"})
    );
}

#[test]
fn tag_with_empty_object_succeeds() {
    let mut h = harness_with_queue();
    let r = h.ok(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {}}));
    assert_eq!(r, json!({"ok": true}));
}

#[test]
fn untag_removes_given_keys_and_ignores_unknown_ones() {
    let mut h = harness_with_queue();
    tag(&mut h, json!({"a": "1", "b": "2", "c": "3"}));
    let r = h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["a", "c", "zz"]}));
    assert_eq!(r, json!({"ok": true}));
    assert_eq!(tags(&mut h), json!({"b": "2"}));
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": []}));
    assert_eq!(tags(&mut h), json!({"b": "2"}));
}

#[test]
fn invalid_tags_are_parameter_errors() {
    let mut h = harness_with_queue();
    let long_key = "k".repeat(129);
    let long_value = "v".repeat(257);
    let bad = [
        json!({"bad key": "v"}),
        json!({"": "v"}),
        json!({long_key: "v"}),
        json!({"k": 1}),
        json!({"k": null}),
        json!({"k": long_value}),
    ];
    for tags in bad {
        let code = h.err(json!({"Command": "TagQueue", "QueueName": "q", "Tags": tags}));
        assert_eq!(code, "InvalidParameterValue", "{tags}");
    }
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "q", "Tags": []})),
        "InvalidParameterValue"
    );
}

#[test]
fn value_limit_is_in_bytes_and_keeps_boundary_values() {
    let mut h = harness_with_queue();
    tag(
        &mut h,
        json!({"ascii": "v".repeat(256), "utf8": "é".repeat(128)}),
    );
    let code = h.err(json!({"Command": "TagQueue", "QueueName": "q",
        "Tags": {"x": "é".repeat(129)}}));
    assert_eq!(code, "InvalidParameterValue");
    assert_eq!(tags(&mut h).as_object().unwrap().len(), 2);
}

#[test]
fn untag_rejects_bad_keys() {
    let mut h = harness_with_queue();
    for keys in [json!(["a b"]), json!([1]), json!(["ok", null]), json!("a")] {
        let code = h.err(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": keys}));
        assert_eq!(code, "InvalidParameterValue", "{keys}");
    }
}

#[test]
fn missing_parameters_and_unknown_queue() {
    let mut h = harness_with_queue();
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "q"})),
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
    for command in ["TagQueue", "UntagQueue", "ListQueueTags"] {
        let code =
            h.err(json!({"Command": command, "QueueName": "nope", "Tags": {}, "TagKeys": []}));
        assert_eq!(code, "QueueDoesNotExist", "{command}");
    }
    // Parameter values are checked before the queue lookup.
    let code = h.err(json!({"Command": "TagQueue", "QueueName": "nope", "Tags": {"a b": "v"}}));
    assert_eq!(code, "InvalidParameterValue");
}

#[test]
fn fifty_tags_fit_and_a_fifty_first_does_not() {
    let mut h = harness_with_queue();
    let fifty: serde_json::Map<String, Value> =
        (0..50).map(|n| (format!("k{n}"), json!("v"))).collect();
    tag(&mut h, Value::Object(fifty));
    tag(&mut h, json!({"k0": "replaced"}));
    let code = h.err(json!({"Command": "TagQueue", "QueueName": "q",
        "Tags": {"k1": "changed", "extra": "v"}}));
    assert_eq!(code, "LimitExceeded");
    let after = tags(&mut h);
    assert_eq!(after.as_object().unwrap().len(), 50);
    assert_eq!(after["k0"], "replaced");
    assert_eq!(after["k1"], "v");
}

#[test]
fn tags_are_not_attributes_and_survive_purge_but_not_delete() {
    let mut h = harness_with_queue();
    tag(&mut h, json!({"a": "1"}));
    let attrs = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(
        attrs["Attributes"],
        json!({
            "VisibilityTimeout": 30, "DelaySeconds": 0, "RetentionSeconds": 345600,
            "Fifo": false, "ContentBasedDeduplication": false, "MaxMessageBytes": 1024,
        })
    );
    // Tags do not take part in CreateQueue rule 4.
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    h.ok(json!({"Command": "PurgeQueue", "QueueName": "q"}));
    assert_eq!(tags(&mut h), json!({"a": "1"}));
    h.ok(json!({"Command": "DeleteQueue", "QueueName": "q"}));
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    assert_eq!(tags(&mut h), json!({}));
}
