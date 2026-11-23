mod common;

use common::Harness;
use serde_json::{Value, json};

fn queue(h: &mut Harness) {
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
}

fn tag(h: &mut Harness, tags: Value) {
    h.ok(json!({"Command": "TagQueue", "QueueName": "q", "Tags": tags}));
}

fn tags(h: &mut Harness, name: &str) -> Value {
    h.ok(json!({"Command": "ListQueueTags", "QueueName": name}))["Tags"].clone()
}

#[test]
fn tags_start_empty_and_are_replaced_per_key() {
    let mut h = Harness::new();
    queue(&mut h);
    assert_eq!(tags(&mut h, "q"), json!({}));
    tag(&mut h, json!({"a": "1", "B": "", "c": "ü"}));
    tag(&mut h, json!({"a": "2"}));
    assert_eq!(tags(&mut h, "q"), json!({"a": "2", "B": "", "c": "ü"}));
}

#[test]
fn untag_removes_listed_keys_and_ignores_unknown_ones() {
    let mut h = Harness::new();
    queue(&mut h);
    tag(&mut h, json!({"a": "1", "b": "2"}));
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["a", "zz"]}));
    assert_eq!(tags(&mut h, "q"), json!({"b": "2"}));
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": []}));
    assert_eq!(tags(&mut h, "q"), json!({"b": "2"}));
}

#[test]
fn tag_value_and_key_rules() {
    let mut h = Harness::new();
    queue(&mut h);
    tag(&mut h, json!({"k".repeat(128): "v".repeat(256)}));
    let bad_tags = [
        json!({"": "v"}),
        json!({"a b": "v"}),
        json!({"k".repeat(129): "v"}),
        json!({"a": "v".repeat(257)}),
        json!({"a": "é".repeat(129)}),
        json!({"a": 1}),
        json!({"a": null}),
    ];
    for bad in bad_tags {
        let c = json!({"Command": "TagQueue", "QueueName": "q", "Tags": bad});
        assert_eq!(h.err(c), "InvalidParameterValue", "{bad}");
    }
    for bad in [json!(["a", 1]), json!([""]), json!(["a b"]), json!("a")] {
        let c = json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": bad});
        assert_eq!(h.err(c), "InvalidParameterValue", "{bad}");
    }
    let c = json!({"Command": "TagQueue", "QueueName": "q", "Tags": []});
    assert_eq!(h.err(c), "InvalidParameterValue");
}

#[test]
fn at_most_fifty_tags_and_a_failed_command_changes_nothing() {
    let mut h = Harness::new();
    queue(&mut h);
    let fifty: serde_json::Map<String, Value> =
        (0..50).map(|i| (format!("k{i}"), json!("v"))).collect();
    tag(&mut h, Value::Object(fifty.clone()));
    // Replacing existing keys stays within the limit.
    tag(&mut h, json!({"k0": "new"}));
    let c = json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"extra": "v", "k1": "x"}});
    assert_eq!(h.err(c), "LimitExceeded");
    let listed = tags(&mut h, "q");
    assert_eq!(listed.as_object().unwrap().len(), 50);
    assert_eq!(listed["k0"], "new");
    assert_eq!(listed["k1"], "v");
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["k0"]}));
    tag(&mut h, json!({"extra": "v"}));
}

#[test]
fn check_order_of_tag_commands() {
    let mut h = Harness::new();
    let cases = [
        json!({"Command": "TagQueue", "QueueName": "x"}),
        json!({"Command": "UntagQueue", "Tags": {}}),
        json!({"Command": "ListQueueTags"}),
    ];
    for case in cases {
        assert_eq!(h.err(case.clone()), "MissingParameter", "{case}");
    }
    // Value checks come before the queue lookup.
    let bad = json!({"Command": "TagQueue", "QueueName": "nope", "Tags": {"a b": "v"}});
    assert_eq!(h.err(bad), "InvalidParameterValue");
    let missing = json!({"Command": "TagQueue", "QueueName": "nope", "Tags": {}});
    assert_eq!(h.err(missing), "QueueDoesNotExist");
    let untag = json!({"Command": "UntagQueue", "QueueName": "nope", "TagKeys": []});
    assert_eq!(h.err(untag), "QueueDoesNotExist");
    assert_eq!(
        h.err(json!({"Command": "ListQueueTags", "QueueName": "nope"})),
        "QueueDoesNotExist"
    );
}

#[test]
fn tags_are_not_attributes_and_survive_purge_but_not_delete() {
    let mut h = Harness::new();
    queue(&mut h);
    tag(&mut h, json!({"a": "1"}));
    let attrs = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert!(attrs["Attributes"].get("Tags").is_none());
    // CreateQueue rule 4 ignores tags.
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    h.ok(json!({"Command": "PurgeQueue", "QueueName": "q"}));
    assert_eq!(tags(&mut h, "q"), json!({"a": "1"}));
    h.ok(json!({"Command": "DeleteQueue", "QueueName": "q"}));
    queue(&mut h);
    assert_eq!(tags(&mut h, "q"), json!({}));
}
