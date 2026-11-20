mod common;

use common::Harness;
use serde_json::{Value, json};

fn create(h: &mut Harness, name: &str) {
    h.ok(json!({"Command": "CreateQueue", "QueueName": name}));
}

fn tag(h: &mut Harness, q: &str, tags: Value) -> Value {
    h.ok(json!({"Command": "TagQueue", "QueueName": q, "Tags": tags}))
}

fn tags(h: &mut Harness, q: &str) -> Value {
    h.ok(json!({"Command": "ListQueueTags", "QueueName": q}))["Tags"].clone()
}

#[test]
fn tag_list_untag() {
    let mut h = Harness::new();
    create(&mut h, "q");
    assert_eq!(tags(&mut h, "q"), json!({}));
    let r = tag(&mut h, "q", json!({"a": "1", "B": "", "c-_9": "é"}));
    assert_eq!(r, json!({"ok": true}));
    assert_eq!(tags(&mut h, "q"), json!({"a": "1", "B": "", "c-_9": "é"}));
    tag(&mut h, "q", json!({"a": "2"}));
    tag(&mut h, "q", json!({}));
    assert_eq!(tags(&mut h, "q")["a"], "2");
    let r = h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["a", "zzz", "b"]}));
    assert_eq!(r, json!({"ok": true}));
    assert_eq!(tags(&mut h, "q"), json!({"B": "", "c-_9": "é"}));
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": []}));
}

#[test]
fn key_and_value_rules() {
    let mut h = Harness::new();
    create(&mut h, "q");
    let long_key = "k".repeat(129);
    let long_value = "v".repeat(257);
    let two_byte_value = "é".repeat(129); // 258 bytes
    for bad in [
        json!({"": "x"}),
        json!({"a b": "x"}),
        json!({"é": "x"}),
        json!({long_key: "x"}),
        json!({"a": 1}),
        json!({"a": null}),
        json!({"a": long_value}),
        json!({"a": two_byte_value}),
        json!({"ok": "x", "bad key": "x"}),
    ] {
        let code = h.err(json!({"Command": "TagQueue", "QueueName": "q", "Tags": bad}));
        assert_eq!(code, "InvalidParameterValue", "{bad}");
    }
    assert_eq!(tags(&mut h, "q"), json!({})); // nothing was applied
    tag(&mut h, "q", json!({"k".repeat(128): "v".repeat(256)}));
    tag(&mut h, "q", json!({"x": "é".repeat(128)}));
    for bad in [json!(null), json!(1), json!([])] {
        let code = h.err(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": [bad]}));
        assert_eq!(code, "InvalidParameterValue");
    }
    let code = h.err(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["a b"]}));
    assert_eq!(code, "InvalidParameterValue");
    assert_eq!(
        h.err(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": {}})),
        "InvalidParameterValue"
    );
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "q", "Tags": []})),
        "InvalidParameterValue"
    );
}

#[test]
fn check_order() {
    let mut h = Harness::new();
    assert_eq!(h.err(json!({"Command": "TagQueue", "QueueName": "q"})), "MissingParameter");
    assert_eq!(h.err(json!({"Command": "TagQueue", "Tags": {}})), "MissingParameter");
    assert_eq!(h.err(json!({"Command": "TagQueue", "QueueName": "q", "Tags": null})), "MissingParameter");
    assert_eq!(h.err(json!({"Command": "UntagQueue", "QueueName": "q"})), "MissingParameter");
    assert_eq!(h.err(json!({"Command": "ListQueueTags"})), "MissingParameter");
    assert_eq!(h.err(json!({"Command": "ListQueueTags", "QueueName": 1})), "InvalidParameterValue");
    // Values (step 3) come before the lookup (step 4).
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"a b": "x"}})),
        "InvalidParameterValue"
    );
    assert_eq!(h.err(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {}})), "QueueDoesNotExist");
    assert_eq!(h.err(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": []})), "QueueDoesNotExist");
    assert_eq!(h.err(json!({"Command": "ListQueueTags", "QueueName": "q"})), "QueueDoesNotExist");
}

#[test]
fn at_most_50_tags() {
    let mut h = Harness::new();
    create(&mut h, "q");
    let fifty: serde_json::Map<String, Value> = (0..50).map(|i| (format!("k{i}"), json!("v"))).collect();
    tag(&mut h, "q", Value::Object(fifty.clone()));
    // Replacing existing keys is fine; a 51st key is not, and changes nothing.
    tag(&mut h, "q", json!({"k0": "new", "k49": "new"}));
    let code = h.err(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"k0": "x", "extra": "y"}}));
    assert_eq!(code, "LimitExceeded");
    let now = tags(&mut h, "q");
    assert_eq!(now["k0"], "new");
    assert!(now.get("extra").is_none());
    assert_eq!(now.as_object().unwrap().len(), 50);
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["k1"]}));
    tag(&mut h, "q", json!({"extra": "y"}));
    // Over the limit in one command from the start.
    create(&mut h, "r");
    let fifty_one: serde_json::Map<String, Value> = (0..51).map(|i| (format!("k{i}"), json!("v"))).collect();
    let code = h.err(json!({"Command": "TagQueue", "QueueName": "r", "Tags": fifty_one}));
    assert_eq!(code, "LimitExceeded");
    assert_eq!(tags(&mut h, "r"), json!({}));
}

#[test]
fn tags_are_not_attributes_and_survive_purge_not_delete() {
    let mut h = Harness::new();
    create(&mut h, "q");
    tag(&mut h, "q", json!({"a": "1"}));
    let r = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert!(r["Attributes"].get("Tags").is_none());
    assert!(r.get("Tags").is_none());
    // CreateQueue rule 4 ignores tags.
    create(&mut h, "q");
    h.ok(json!({"Command": "SendMessage", "QueueName": "q", "Body": "x"}));
    h.ok(json!({"Command": "PurgeQueue", "QueueName": "q"}));
    assert_eq!(tags(&mut h, "q"), json!({"a": "1"}));
    h.ok(json!({"Command": "DeleteQueue", "QueueName": "q"}));
    create(&mut h, "q");
    assert_eq!(tags(&mut h, "q"), json!({}));
}

#[test]
fn tags_are_per_queue() {
    let mut h = Harness::new();
    create(&mut h, "a");
    create(&mut h, "b");
    tag(&mut h, "a", json!({"k": "v"}));
    assert_eq!(tags(&mut h, "b"), json!({}));
}
