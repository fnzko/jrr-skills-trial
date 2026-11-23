mod common;

use common::Harness;
use serde_json::{Value, json};

fn tags(h: &mut Harness, q: &str) -> Value {
    h.ok(json!({"Command": "ListQueueTags", "QueueName": q}))["Tags"].clone()
}

fn setup() -> Harness {
    let mut h = Harness::new();
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    h
}

#[test]
fn tag_list_untag() {
    let mut h = setup();
    assert_eq!(tags(&mut h, "q"), json!({}));
    h.ok(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"a": "1", "B": ""}}));
    h.ok(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"a": "2"}}));
    assert_eq!(tags(&mut h, "q"), json!({"a": "2", "B": ""}));
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["a", "zz"]}));
    assert_eq!(tags(&mut h, "q"), json!({"B": ""}));
    h.ok(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {}}));
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": []}));
    // Response has no extra members.
    assert_eq!(h.run(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {}})), json!({"ok": true}));
}

#[test]
fn validation_and_order() {
    let mut h = setup();
    let t = |tags: Value| json!({"Command": "TagQueue", "QueueName": "q", "Tags": tags});
    assert_eq!(h.err(json!({"Command": "TagQueue", "QueueName": "q"})), "MissingParameter");
    assert_eq!(h.err(json!({"Command": "UntagQueue", "QueueName": "q"})), "MissingParameter");
    assert_eq!(h.err(t(json!([]))), "InvalidParameterValue");
    assert_eq!(h.err(t(json!({"bad key": "v"}))), "InvalidParameterValue");
    assert_eq!(h.err(t(json!({"": "v"}))), "InvalidParameterValue");
    assert_eq!(h.err(t(json!({"k".repeat(129): "v"}))), "InvalidParameterValue");
    h.ok(t(json!({"k".repeat(128): "v"})));
    assert_eq!(h.err(t(json!({"k": 1}))), "InvalidParameterValue");
    assert_eq!(h.err(t(json!({"k": null}))), "InvalidParameterValue");
    assert_eq!(h.err(t(json!({"k": "x".repeat(257)}))), "InvalidParameterValue");
    // 256 bytes: 128 two-byte chars OK, 129 not.
    h.ok(t(json!({"k": "é".repeat(128)})));
    assert_eq!(h.err(t(json!({"k": "é".repeat(129)}))), "InvalidParameterValue");
    // One bad tag means nothing is set.
    assert_eq!(h.err(t(json!({"good": "v", "bad!": "v"}))), "InvalidParameterValue");
    assert!(tags(&mut h, "q").get("good").is_none());
    for cmd in [
        json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": "a"}),
        json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": [1]}),
        json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["a b"]}),
    ] {
        assert_eq!(h.err(cmd), "InvalidParameterValue");
    }
    // Step 3 before lookup (step 4).
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "nope", "Tags": 3})),
        "InvalidParameterValue"
    );
    for c in ["TagQueue", "UntagQueue", "ListQueueTags"] {
        assert_eq!(
            h.err(json!({"Command": c, "QueueName": "nope", "Tags": {}, "TagKeys": []})),
            "QueueDoesNotExist"
        );
    }
}

#[test]
fn at_most_fifty_tags() {
    let mut h = setup();
    let many = |range: std::ops::Range<u32>| {
        Value::Object(range.map(|i| (format!("k{i}"), json!("v"))).collect())
    };
    h.ok(json!({"Command": "TagQueue", "QueueName": "q", "Tags": many(0..50)}));
    // Replacing existing keys is fine; a 51st key is not, and nothing is set.
    h.ok(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"k0": "new"}}));
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"k0": "x", "extra": "y"}})),
        "LimitExceeded"
    );
    let t = tags(&mut h, "q");
    assert_eq!(t.as_object().unwrap().len(), 50);
    assert_eq!(t["k0"], "new");
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "q", "Tags": many(0..51)})),
        "LimitExceeded"
    );
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["k1"]}));
    h.ok(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"extra": "y"}}));
}

#[test]
fn tags_are_not_attributes_and_follow_the_queue() {
    let mut h = setup();
    h.ok(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"a": "1"}}));
    let attrs = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}))["Attributes"].clone();
    assert!(attrs.get("Tags").is_none());
    // CreateQueue rule 4 ignores tags.
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    h.ok(json!({"Command": "SendMessage", "QueueName": "q", "Body": "x"}));
    h.ok(json!({"Command": "PurgeQueue", "QueueName": "q"}));
    assert_eq!(tags(&mut h, "q"), json!({"a": "1"}));
    h.ok(json!({"Command": "DeleteQueue", "QueueName": "q"}));
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    assert_eq!(tags(&mut h, "q"), json!({}));
}
