mod common;

use common::Harness;
use serde_json::{Value, json};

fn tag(h: &mut Harness, tags: Value) -> Value {
    h.run(json!({"Command": "TagQueue", "QueueName": "q", "Tags": tags}))
}

fn list(h: &mut Harness) -> Value {
    h.ok(json!({"Command": "ListQueueTags", "QueueName": "q"}))["Tags"].clone()
}

fn harness() -> Harness {
    let mut h = Harness::new();
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    h
}

#[test]
fn tag_list_replace_untag() {
    let mut h = harness();
    assert_eq!(list(&mut h), json!({}));
    assert_eq!(tag(&mut h, json!({}))["ok"], true);
    assert_eq!(tag(&mut h, json!({"a": "1", "B": "", "c_-": "é"}))["ok"], true);
    assert_eq!(list(&mut h), json!({"a": "1", "B": "", "c_-": "é"}));
    tag(&mut h, json!({"a": "2", "b": "x"}));
    assert_eq!(list(&mut h), json!({"a": "2", "B": "", "b": "x", "c_-": "é"}));
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["a", "nope", "A"]}));
    assert_eq!(list(&mut h), json!({"B": "", "b": "x", "c_-": "é"}));
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": []}));
    assert_eq!(list(&mut h).as_object().unwrap().len(), 3);
}

#[test]
fn validation() {
    let mut h = harness();
    let e = |h: &mut Harness, c: Value| h.err(c);
    assert_eq!(e(&mut h, json!({"Command": "TagQueue", "QueueName": "q"})), "MissingParameter");
    assert_eq!(e(&mut h, json!({"Command": "TagQueue", "Tags": {}})), "MissingParameter");
    assert_eq!(e(&mut h, json!({"Command": "UntagQueue", "QueueName": "q"})), "MissingParameter");
    assert_eq!(e(&mut h, json!({"Command": "ListQueueTags"})), "MissingParameter");
    assert_eq!(e(&mut h, json!({"Command": "TagQueue", "QueueName": "q", "Tags": []})), "InvalidParameterValue");
    for bad in [json!({"a b": "1"}), json!({"": "1"}), json!({"k".repeat(129): "1"}),
        json!({"a": 1}), json!({"a": null}), json!({"a": "v".repeat(257)}), json!({"a": "é".repeat(129)})] {
        assert_eq!(e(&mut h, json!({"Command": "TagQueue", "QueueName": "q", "Tags": bad})), "InvalidParameterValue");
    }
    for bad in [json!({}), json!(["a b"]), json!([1]), json!([""]), json!(["a", null])] {
        assert_eq!(e(&mut h, json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": bad})), "InvalidParameterValue");
    }
    // Step 3 comes before the lookup.
    assert_eq!(e(&mut h, json!({"Command": "TagQueue", "QueueName": "zz", "Tags": {"a b": "1"}})), "InvalidParameterValue");
    for c in ["TagQueue", "UntagQueue", "ListQueueTags"] {
        assert_eq!(e(&mut h, json!({"Command": c, "QueueName": "zz", "Tags": {}, "TagKeys": []})), "QueueDoesNotExist");
    }
    // Boundary values are accepted and nothing was changed by failures.
    assert_eq!(tag(&mut h, json!({"k".repeat(128): "v".repeat(256)}))["ok"], true);
    assert_eq!(tag(&mut h, json!({"é": "x"}))["ok"], false);
    assert_eq!(list(&mut h).as_object().unwrap().len(), 1);
}

#[test]
fn fifty_tag_limit() {
    let mut h = harness();
    let many = |lo: u32, hi: u32| Value::Object((lo..hi).map(|i| (format!("k{i}"), json!("v"))).collect());
    assert_eq!(tag(&mut h, many(0, 50))["ok"], true);
    assert_eq!(tag(&mut h, many(0, 51))["error"]["code"], "LimitExceeded");
    assert_eq!(list(&mut h).as_object().unwrap().len(), 50);
    // Replacing existing keys does not count; nothing partial is applied.
    assert_eq!(tag(&mut h, json!({"k0": "new"}))["ok"], true);
    assert_eq!(tag(&mut h, json!({"k0": "newer", "x": "1"}))["error"]["code"], "LimitExceeded");
    assert_eq!(list(&mut h)["k0"], "new");
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["k1"]}));
    assert_eq!(tag(&mut h, json!({"x": "1"}))["ok"], true);
    // More than 50 in one command on an empty queue.
    h.ok(json!({"Command": "CreateQueue", "QueueName": "r"}));
    let r = h.run(json!({"Command": "TagQueue", "QueueName": "r", "Tags": many(0, 51)}));
    assert_eq!(r["error"]["code"], "LimitExceeded");
}

#[test]
fn tags_are_not_attributes_and_survive_purge_not_delete() {
    let mut h = harness();
    tag(&mut h, json!({"a": "1"}));
    let a = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert!(a["Attributes"].get("Tags").is_none());
    // CreateQueue rule 4 ignores tags.
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    h.ok(json!({"Command": "SendMessage", "QueueName": "q", "Body": "x"}));
    h.ok(json!({"Command": "PurgeQueue", "QueueName": "q"}));
    assert_eq!(list(&mut h), json!({"a": "1"}));
    h.ok(json!({"Command": "DeleteQueue", "QueueName": "q"}));
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    assert_eq!(list(&mut h), json!({}));
}
