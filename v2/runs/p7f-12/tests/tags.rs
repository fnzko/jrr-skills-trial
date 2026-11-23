mod common;

use common::Harness;
use serde_json::json;

fn harness_with_queue() -> Harness {
    let mut h = Harness::new();
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    h
}

fn tags(h: &mut Harness) -> serde_json::Value {
    h.ok(json!({"Command": "ListQueueTags", "QueueName": "q"}))["Tags"].clone()
}

#[test]
fn tag_list_replace_and_untag() {
    let mut h = harness_with_queue();
    assert_eq!(tags(&mut h), json!({}));
    h.ok(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"a": "1", "B": ""}}));
    h.ok(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"a": "2", "b": "x"}}));
    assert_eq!(tags(&mut h), json!({"a": "2", "B": "", "b": "x"}));
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["a", "zzz"]}));
    assert_eq!(tags(&mut h), json!({"B": "", "b": "x"}));
    h.ok(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {}}));
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": []}));
    assert_eq!(tags(&mut h), json!({"B": "", "b": "x"}));
}

#[test]
fn tag_value_and_key_rules() {
    let mut h = harness_with_queue();
    let tag = |t| json!({"Command": "TagQueue", "QueueName": "q", "Tags": t});
    h.ok(tag(json!({"k": "x".repeat(256), "e".repeat(128): "é".repeat(128)})));
    for bad in [
        json!({"k": "x".repeat(257)}),
        json!({"k": "é".repeat(129)}),
        json!({"k": 1}),
        json!({"k": null}),
        json!({"": "v"}),
        json!({"a b": "v"}),
        json!({"e".repeat(129): "v"}),
    ] {
        assert_eq!(h.err(tag(bad)), "InvalidParameterValue");
    }
    for bad in [json!([1]), json!([""]), json!(["a b"]), json!("a")] {
        assert_eq!(
            h.err(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": bad})),
            "InvalidParameterValue"
        );
    }
    assert_eq!(h.err(tag(json!([]))), "InvalidParameterValue");
}

#[test]
fn tag_limit_counts_keys_after_the_command() {
    let mut h = harness_with_queue();
    let fifty: serde_json::Map<_, _> = (0..50).map(|i| (format!("k{i}"), json!("v"))).collect();
    h.ok(json!({"Command": "TagQueue", "QueueName": "q", "Tags": fifty}));
    // replacing existing keys is fine, a 51st key is not, and nothing is applied
    h.ok(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"k0": "new"}}));
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"k1": "new", "extra": "v"}})),
        "LimitExceeded"
    );
    let now = tags(&mut h);
    assert_eq!(now["k0"], "new");
    assert_eq!(now["k1"], "v");
    assert_eq!(now.as_object().unwrap().len(), 50);
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["k2"]}));
    h.ok(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"extra": "v"}}));
}

#[test]
fn tag_check_order() {
    let mut h = harness_with_queue();
    for command in ["TagQueue", "UntagQueue", "ListQueueTags"] {
        assert_eq!(h.err(json!({"Command": command})), "MissingParameter");
        assert_eq!(
            h.err(json!({"Command": command, "QueueName": "nope", "Tags": {}, "TagKeys": []})),
            "QueueDoesNotExist"
        );
    }
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "q"})),
        "MissingParameter"
    );
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "nope", "Tags": {"a b": "v"}})),
        "InvalidParameterValue"
    );
}

#[test]
fn tags_are_not_attributes_and_survive_purge_but_not_delete() {
    let mut h = harness_with_queue();
    h.ok(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"a": "1"}}));
    let attrs = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert!(attrs["Attributes"].get("Tags").is_none());
    // CreateQueue rule 4 ignores tags
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    h.ok(json!({"Command": "PurgeQueue", "QueueName": "q"}));
    assert_eq!(tags(&mut h), json!({"a": "1"}));
    h.ok(json!({"Command": "DeleteQueue", "QueueName": "q"}));
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    assert_eq!(tags(&mut h), json!({}));
}
