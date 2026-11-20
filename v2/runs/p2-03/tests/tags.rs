mod common;

use common::Harness;
use serde_json::{Value, json};

fn queue(h: &mut Harness) {
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
}

fn tag(h: &mut Harness, tags: Value) {
    h.ok(json!({"Command": "TagQueue", "QueueName": "q", "Tags": tags}));
}

fn tags(h: &mut Harness) -> Value {
    h.ok(json!({"Command": "ListQueueTags", "QueueName": "q"}))["Tags"].clone()
}

#[test]
fn new_queue_has_no_tags() {
    let mut h = Harness::new();
    queue(&mut h);
    assert_eq!(tags(&mut h), json!({}));
}

#[test]
fn tag_sets_replaces_and_untag_removes() {
    let mut h = Harness::new();
    queue(&mut h);
    tag(&mut h, json!({"env": "prod", "Team": "a", "empty": ""}));
    tag(&mut h, json!({"env": "dev"}));
    assert_eq!(
        tags(&mut h),
        json!({"env": "dev", "Team": "a", "empty": ""})
    );
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["Team", "absent", "env"]}));
    assert_eq!(tags(&mut h), json!({"empty": ""}));
    tag(&mut h, json!({}));
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": []}));
    assert_eq!(tags(&mut h), json!({"empty": ""}));
}

#[test]
fn keys_are_case_sensitive() {
    let mut h = Harness::new();
    queue(&mut h);
    tag(&mut h, json!({"a": "1", "A": "2"}));
    assert_eq!(tags(&mut h), json!({"a": "1", "A": "2"}));
}

#[test]
fn key_and_value_limits() {
    let mut h = Harness::new();
    queue(&mut h);
    tag(&mut h, json!({"k".repeat(128): "v".repeat(256)}));
    // 128 two-byte characters are 256 bytes; 129 are not allowed.
    tag(&mut h, json!({"u": "é".repeat(128)}));
    let bad = [
        json!({"": "v"}),
        json!({"k".repeat(129): "v"}),
        json!({"a b": "v"}),
        json!({"k": "v".repeat(257)}),
        json!({"k": "é".repeat(129)}),
        json!({"k": 1}),
        json!({"k": null}),
        json!({"ok": "v", "bad key": "v"}),
    ];
    for tags in bad {
        assert_eq!(
            h.err(json!({"Command": "TagQueue", "QueueName": "q", "Tags": tags.clone()})),
            "InvalidParameterValue",
            "{tags}"
        );
    }
    // A failed command changes nothing.
    assert_eq!(tags(&mut h).as_object().unwrap().len(), 2);
}

#[test]
fn untag_keys_are_validated() {
    let mut h = Harness::new();
    queue(&mut h);
    tag(&mut h, json!({"a": "1"}));
    for keys in [
        json!(["a", 1]),
        json!(["a", "b c"]),
        json!([""]),
        json!(["a", null]),
    ] {
        assert_eq!(
            h.err(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": keys})),
            "InvalidParameterValue"
        );
    }
    assert_eq!(
        h.err(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": "a"})),
        "InvalidParameterValue"
    );
    assert_eq!(tags(&mut h), json!({"a": "1"}));
}

#[test]
fn at_most_fifty_tags() {
    let mut h = Harness::new();
    queue(&mut h);
    let fifty: serde_json::Map<String, Value> =
        (0..50).map(|i| (format!("k{i}"), json!("v"))).collect();
    tag(&mut h, Value::Object(fifty));
    // Replacing an existing key adds nothing; a 51st key is refused whole.
    tag(&mut h, json!({"k0": "new"}));
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"k0": "x", "extra": "y"}})),
        "LimitExceeded"
    );
    let t = tags(&mut h);
    assert_eq!(t.as_object().unwrap().len(), 50);
    assert_eq!(t["k0"], "new");
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["k1"]}));
    tag(&mut h, json!({"extra": "y"}));
    // 51 distinct keys in one command on an empty queue also exceed the limit.
    h.ok(json!({"Command": "CreateQueue", "QueueName": "r"}));
    let fifty_one: serde_json::Map<String, Value> =
        (0..51).map(|i| (format!("k{i}"), json!("v"))).collect();
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "r", "Tags": fifty_one})),
        "LimitExceeded"
    );
}

#[test]
fn check_order() {
    let mut h = Harness::new();
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "q"})),
        "MissingParameter"
    );
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "Tags": {}})),
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
    // Value errors (step 3) come before the lookup (step 4).
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"a b": "v"}})),
        "InvalidParameterValue"
    );
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "q", "Tags": []})),
        "InvalidParameterValue"
    );
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"a": "v"}})),
        "QueueDoesNotExist"
    );
    assert_eq!(
        h.err(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": []})),
        "QueueDoesNotExist"
    );
    assert_eq!(
        h.err(json!({"Command": "ListQueueTags", "QueueName": "q"})),
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
    h.ok(json!({"Command": "SendMessage", "QueueName": "q", "Body": "x"}));
    h.ok(json!({"Command": "PurgeQueue", "QueueName": "q"}));
    assert_eq!(tags(&mut h), json!({"a": "1"}));
    h.ok(json!({"Command": "DeleteQueue", "QueueName": "q"}));
    queue(&mut h);
    assert_eq!(tags(&mut h), json!({}));
}
