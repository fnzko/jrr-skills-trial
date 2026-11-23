mod common;

use common::Harness;
use serde_json::json;

fn setup() -> Harness {
    let mut h = Harness::new();
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    h
}

fn tags(h: &mut Harness) -> serde_json::Value {
    h.ok(json!({"Command": "ListQueueTags", "QueueName": "q"}))["Tags"].clone()
}

#[test]
fn tag_list_replace_untag() {
    let mut h = setup();
    assert_eq!(tags(&mut h), json!({}));
    h.ok(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"a": "1", "B": ""}}));
    h.ok(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"a": "2", "c": "é"}}));
    assert_eq!(tags(&mut h), json!({"a": "2", "B": "", "c": "é"}));
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["a", "zzz", "b"]}));
    assert_eq!(tags(&mut h), json!({"B": "", "c": "é"}));
    h.ok(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {}}));
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": []}));
    assert_eq!(tags(&mut h), json!({"B": "", "c": "é"}));
}

#[test]
fn validation_and_check_order() {
    let mut h = setup();
    let t =
        |tags: serde_json::Value| json!({"Command": "TagQueue", "QueueName": "q", "Tags": tags});
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "q"})),
        "MissingParameter"
    );
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "q", "Tags": null})),
        "MissingParameter"
    );
    assert_eq!(
        h.err(json!({"Command": "UntagQueue", "QueueName": "q"})),
        "MissingParameter"
    );
    for bad in [
        json!([]),
        json!("x"),
        json!({"": "v"}),
        json!({"a b": "v"}),
        json!({"a": 1}),
        json!({"a": null}),
        json!({"k".repeat(129): "v"}),
        json!({"a": "x".repeat(257)}),
        json!({"a": "é".repeat(129)}),
    ] {
        assert_eq!(h.err(t(bad.clone())), "InvalidParameterValue", "{bad}");
    }
    h.ok(t(
        json!({"k".repeat(128): "x".repeat(256), "b": "é".repeat(128)}),
    ));
    for bad in [
        json!({}),
        json!("a"),
        json!([1]),
        json!([""]),
        json!(["a b"]),
    ] {
        assert_eq!(
            h.err(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": bad})),
            "InvalidParameterValue",
            "{bad}"
        );
    }
    // Value errors (step 3) come before the queue lookup (step 4).
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "nope", "Tags": {"a": 1}})),
        "InvalidParameterValue"
    );
    for cmd in ["TagQueue", "UntagQueue", "ListQueueTags"] {
        assert_eq!(
            h.err(json!({"Command": cmd, "QueueName": "nope", "Tags": {}, "TagKeys": []})),
            "QueueDoesNotExist"
        );
    }
}

#[test]
fn at_most_fifty_tags() {
    let mut h = setup();
    let many = |range: std::ops::Range<u32>| {
        let m: serde_json::Map<_, _> = range.map(|i| (format!("k{i}"), json!("v"))).collect();
        json!({"Command": "TagQueue", "QueueName": "q", "Tags": m})
    };
    h.ok(many(0..50));
    // Replacing existing keys is fine; a 51st key is not, and changes nothing.
    h.ok(many(0..50));
    assert_eq!(h.err(many(49..51)), "LimitExceeded");
    assert_eq!(tags(&mut h).as_object().unwrap().len(), 50);
    assert_eq!(h.err(many(0..51)), "LimitExceeded");
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["k0"]}));
    h.ok(many(50..51));
    assert_eq!(h.err(many(0..1)), "LimitExceeded");
}

#[test]
fn tags_are_not_attributes() {
    let mut h = setup();
    h.ok(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"a": "1"}}));
    let a = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert!(a["Attributes"].get("Tags").is_none());
    // Idempotent CreateQueue ignores tags and PurgeQueue keeps them.
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    h.ok(json!({"Command": "PurgeQueue", "QueueName": "q"}));
    assert_eq!(tags(&mut h), json!({"a": "1"}));
    // A deleted queue's tags are gone with it.
    h.ok(json!({"Command": "DeleteQueue", "QueueName": "q"}));
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    assert_eq!(tags(&mut h), json!({}));
}
