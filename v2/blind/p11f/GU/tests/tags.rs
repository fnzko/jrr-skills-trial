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

fn tag(h: &mut Harness, t: Value) -> String {
    h.err(json!({"Command": "TagQueue", "QueueName": "q", "Tags": t}))
}

#[test]
fn tag_list_replace_untag() {
    let mut h = setup();
    assert_eq!(tags(&mut h), json!({}));
    h.ok(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"a": "1", "B": ""}}));
    h.ok(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"a": "2", "b": "x"}}));
    assert_eq!(tags(&mut h), json!({"a": "2", "B": "", "b": "x"}));
    h.ok(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {}}));
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["a", "zzz"]}));
    assert_eq!(tags(&mut h), json!({"B": "", "b": "x"}));
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": []}));
}

#[test]
fn parameter_errors() {
    let mut h = setup();
    assert_eq!(h.err(json!({"Command": "TagQueue", "QueueName": "q"})), "MissingParameter");
    assert_eq!(h.err(json!({"Command": "TagQueue", "Tags": {}})), "MissingParameter");
    assert_eq!(h.err(json!({"Command": "UntagQueue", "QueueName": "q"})), "MissingParameter");
    assert_eq!(h.err(json!({"Command": "ListQueueTags"})), "MissingParameter");
    assert_eq!(tag(&mut h, json!([])), "InvalidParameterValue");
    assert_eq!(tag(&mut h, json!({"a b": "x"})), "InvalidParameterValue");
    assert_eq!(tag(&mut h, json!({"": "x"})), "InvalidParameterValue");
    assert_eq!(tag(&mut h, json!({"a": 1})), "InvalidParameterValue");
    assert_eq!(tag(&mut h, json!({"a": null})), "InvalidParameterValue");
    let long_key = "k".repeat(129);
    assert_eq!(tag(&mut h, json!({ long_key: "x" })), "InvalidParameterValue");
    h.ok(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"k".repeat(128): "é".repeat(128)}}));
    assert_eq!(tag(&mut h, json!({"a": "é".repeat(128) + "x"})), "InvalidParameterValue");
    for bad in [json!("a"), json!([1]), json!(["a b"]), json!([""])] {
        assert_eq!(
            h.err(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": bad})),
            "InvalidParameterValue"
        );
    }
    // Step 3 comes before the queue lookup, which comes before the limit.
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "nope", "Tags": {"a": 1}})),
        "InvalidParameterValue"
    );
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "nope", "Tags": {}})),
        "QueueDoesNotExist"
    );
    assert_eq!(
        h.err(json!({"Command": "UntagQueue", "QueueName": "nope", "TagKeys": []})),
        "QueueDoesNotExist"
    );
    assert_eq!(h.err(json!({"Command": "ListQueueTags", "QueueName": "nope"})), "QueueDoesNotExist");
    // A failed command changed nothing (only the valid long tag exists).
    assert_eq!(tags(&mut h).as_object().unwrap().len(), 1);
}

#[test]
fn fifty_tag_limit() {
    let mut h = setup();
    let make = |range: std::ops::Range<u32>| -> Value {
        Value::Object(range.map(|i| (format!("k{i}"), json!("v"))).collect())
    };
    h.ok(json!({"Command": "TagQueue", "QueueName": "q", "Tags": make(0..50)}));
    assert_eq!(tag(&mut h, make(50..51)), "LimitExceeded");
    // Replacing existing keys is fine at the limit, and a mix that overflows changes nothing.
    h.ok(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"k0": "new"}}));
    assert_eq!(tag(&mut h, json!({"k1": "new", "extra": "x"})), "LimitExceeded");
    assert_eq!(tags(&mut h)["k1"], "v");
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["k0"]}));
    h.ok(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"extra": "x"}}));
    assert_eq!(tags(&mut h).as_object().unwrap().len(), 50);
    assert_eq!(tag(&mut h, make(100..151)), "LimitExceeded");
}

#[test]
fn tags_are_not_attributes() {
    let mut h = setup();
    h.ok(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"a": "1"}}));
    let r = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert!(r["Attributes"].get("Tags").is_none());
    // CreateQueue rule 4 ignores tags; Purge keeps them.
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    h.ok(json!({"Command": "PurgeQueue", "QueueName": "q"}));
    assert_eq!(tags(&mut h), json!({"a": "1"}));
    // A deleted queue's tags are gone.
    h.ok(json!({"Command": "DeleteQueue", "QueueName": "q"}));
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    assert_eq!(tags(&mut h), json!({}));
}
