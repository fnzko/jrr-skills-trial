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

fn list(h: &mut Harness) -> Value {
    h.ok(json!({"Command": "ListQueueTags", "QueueName": "q"}))["Tags"].clone()
}

#[test]
fn set_replace_remove_list() {
    let mut h = setup();
    assert_eq!(list(&mut h), json!({}));
    h.ok(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"a": "1", "b": ""}}));
    h.ok(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"a": "2", "C": "x"}}));
    assert_eq!(list(&mut h), json!({"a": "2", "b": "", "C": "x"}));
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["a", "missing", "c"]}));
    assert_eq!(list(&mut h), json!({"b": "", "C": "x"}));
    h.ok(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {}}));
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": []}));
}

#[test]
fn parameter_checks() {
    let mut h = setup();
    assert_eq!(h.err(json!({"Command": "TagQueue", "QueueName": "q"})), "MissingParameter");
    assert_eq!(h.err(json!({"Command": "TagQueue", "Tags": {}})), "MissingParameter");
    assert_eq!(h.err(json!({"Command": "UntagQueue", "QueueName": "q"})), "MissingParameter");
    assert_eq!(h.err(json!({"Command": "ListQueueTags"})), "MissingParameter");
    for bad in [json!([]), json!("x"), json!({"": "v"}), json!({"a b": "v"}),
        json!({"k".repeat(129): "v"}), json!({"k": 1}), json!({"k": "é".repeat(129)}),
        json!({"k": "x".repeat(257)})] {
        assert_eq!(tag(&mut h, bad.clone())["error"]["code"], "InvalidParameterValue", "{bad}");
    }
    for bad in [json!({}), json!("a"), json!([1]), json!(["a b"]), json!([""]), json!(["a", null])] {
        assert_eq!(
            h.err(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": bad.clone()})),
            "InvalidParameterValue", "{bad}"
        );
    }
    assert_eq!(list(&mut h), json!({}));
    // boundaries are fine
    assert_eq!(tag(&mut h, json!({"k".repeat(128): "é".repeat(128)}))["ok"], true);
    assert_eq!(tag(&mut h, json!({"v": "x".repeat(256)}))["ok"], true);
}

#[test]
fn value_check_precedes_queue_lookup() {
    let mut h = Harness::new();
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "nope", "Tags": {"a b": "x"}})),
        "InvalidParameterValue"
    );
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "nope", "Tags": {}})),
        "QueueDoesNotExist"
    );
    assert_eq!(h.err(json!({"Command": "ListQueueTags", "QueueName": "nope"})), "QueueDoesNotExist");
    assert_eq!(
        h.err(json!({"Command": "UntagQueue", "QueueName": "nope", "TagKeys": []})),
        "QueueDoesNotExist"
    );
}

#[test]
fn limit_of_fifty() {
    let mut h = setup();
    let many = |range: std::ops::Range<u32>| -> Value {
        range.map(|i| (format!("k{i}"), json!("v"))).collect::<serde_json::Map<_, _>>().into()
    };
    assert_eq!(tag(&mut h, many(0..51))["error"]["code"], "LimitExceeded");
    assert_eq!(list(&mut h), json!({}));
    assert_eq!(tag(&mut h, many(0..50))["ok"], true);
    // replacing existing keys is fine, a new one is not
    assert_eq!(tag(&mut h, json!({"k0": "new"}))["ok"], true);
    assert_eq!(tag(&mut h, json!({"k0": "new", "extra": "x"}))["error"]["code"], "LimitExceeded");
    assert_eq!(list(&mut h)["k0"], "new");
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["k1"]}));
    assert_eq!(tag(&mut h, json!({"extra": "x"}))["ok"], true);
}

#[test]
fn tags_are_not_attributes_and_survive_purge_not_delete() {
    let mut h = setup();
    h.ok(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"a": "1"}}));
    let r = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert!(r["Attributes"].get("Tags").is_none());
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"})); // idempotent despite tags
    h.ok(json!({"Command": "SendMessage", "QueueName": "q", "Body": "x"}));
    h.ok(json!({"Command": "PurgeQueue", "QueueName": "q"}));
    assert_eq!(list(&mut h), json!({"a": "1"}));
    h.ok(json!({"Command": "DeleteQueue", "QueueName": "q"}));
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    assert_eq!(list(&mut h), json!({}));
}
