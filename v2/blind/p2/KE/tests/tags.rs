mod common;

use common::Harness;
use serde_json::{Value, json};

fn setup() -> Harness {
    let mut h = Harness::new();
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    h
}

fn tag(h: &mut Harness, tags: Value) -> Value {
    h.run(json!({"Command": "TagQueue", "QueueName": "q", "Tags": tags}))
}

fn tags(h: &mut Harness) -> Value {
    h.ok(json!({"Command": "ListQueueTags", "QueueName": "q"}))["Tags"].clone()
}

#[test]
fn none_at_first_and_set_replace_untag() {
    let mut h = setup();
    assert_eq!(tags(&mut h), json!({}));
    assert_eq!(tag(&mut h, json!({"a": "1", "b": ""}))["ok"], true);
    assert_eq!(tag(&mut h, json!({"a": "2"}))["ok"], true);
    assert_eq!(tags(&mut h), json!({"a": "2", "b": ""}));
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["a", "zzz"]}));
    assert_eq!(tags(&mut h), json!({"b": ""}));
    assert_eq!(tag(&mut h, json!({}))["ok"], true);
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": []}));
}

#[test]
fn validation() {
    let mut h = setup();
    for bad in [
        json!({"bad key": "v"}),
        json!({"": "v"}),
        json!({"k": 1}),
        json!({"k": null}),
        json!({"k": "x".repeat(257)}),
        json!({"a".repeat(129): "v"}),
    ] {
        assert_eq!(tag(&mut h, bad)["error"]["code"], "InvalidParameterValue");
    }
    assert_eq!(tag(&mut h, json!({"k": "x".repeat(256)}))["ok"], true);
    assert_eq!(tag(&mut h, json!({"a".repeat(128): "é".repeat(128)}))["ok"], true);
    assert_eq!(tag(&mut h, json!({"j": "é".repeat(129)}))["ok"], false);
    for bad in [json!([1]), json!(["a b"]), json!("a")] {
        let c = json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": bad});
        assert_eq!(h.err(c), "InvalidParameterValue");
    }
    assert_eq!(tags(&mut h).as_object().unwrap().len(), 2);
}

#[test]
fn keys_are_case_sensitive() {
    let mut h = setup();
    tag(&mut h, json!({"a": "1", "A": "2"}));
    assert_eq!(tags(&mut h), json!({"a": "1", "A": "2"}));
}

#[test]
fn error_order() {
    let mut h = Harness::new();
    assert_eq!(h.err(json!({"Command": "TagQueue", "QueueName": "nope"})), "MissingParameter");
    assert_eq!(h.err(json!({"Command": "UntagQueue", "QueueName": "nope"})), "MissingParameter");
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "nope", "Tags": {"x y": "v"}})),
        "InvalidParameterValue"
    );
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "nope", "Tags": {}})),
        "QueueDoesNotExist"
    );
    assert_eq!(h.err(json!({"Command": "ListQueueTags", "QueueName": "nope"})), "QueueDoesNotExist");
}

#[test]
fn limit_of_fifty() {
    let mut h = setup();
    let fifty: serde_json::Map<String, Value> = (0..50).map(|i| (format!("k{i}"), json!("v"))).collect();
    assert_eq!(tag(&mut h, Value::Object(fifty))["ok"], true);
    assert_eq!(tag(&mut h, json!({"extra": "v"}))["error"]["code"], "LimitExceeded");
    assert_eq!(tag(&mut h, json!({"k0": "new"}))["ok"], true); // replacement is fine
    assert_eq!(tags(&mut h).as_object().unwrap().len(), 50);
    // new + replaced mix that would exceed: nothing changes
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["k1"]}));
    assert_eq!(tag(&mut h, json!({"n1": "v", "n2": "v", "k0": "z"}))["error"]["code"], "LimitExceeded");
    assert_eq!(tags(&mut h)["k0"], "new");
    assert!(tags(&mut h).get("n1").is_none());
}

#[test]
fn tags_are_not_attributes_survive_purge_and_die_with_queue() {
    let mut h = setup();
    tag(&mut h, json!({"a": "1"}));
    let r = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert!(r["Attributes"].get("a").is_none() && r["Attributes"].get("Tags").is_none());
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"})); // rule 4 ignores tags
    h.ok(json!({"Command": "PurgeQueue", "QueueName": "q"}));
    assert_eq!(tags(&mut h), json!({"a": "1"}));
    h.ok(json!({"Command": "DeleteQueue", "QueueName": "q"}));
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    assert_eq!(tags(&mut h), json!({}));
}
