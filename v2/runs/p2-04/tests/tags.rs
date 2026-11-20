mod common;

use common::Harness;
use serde_json::{Value, json};

fn tags(h: &mut Harness, queue: &str) -> Value {
    h.ok(json!({"Command": "ListQueueTags", "QueueName": queue}))["Tags"].clone()
}

fn tag(h: &mut Harness, queue: &str, t: Value) {
    h.ok(json!({"Command": "TagQueue", "QueueName": queue, "Tags": t}));
}

fn queue(h: &mut Harness, name: &str) {
    h.ok(json!({"Command": "CreateQueue", "QueueName": name}));
}

#[test]
fn tag_list_replace_and_untag() {
    let mut h = Harness::new();
    queue(&mut h, "q");
    assert_eq!(tags(&mut h, "q"), json!({}));
    assert_eq!(
        h.ok(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"a": "1", "B": ""}})),
        json!({"ok": true})
    );
    tag(&mut h, "q", json!({"a": "2"}));
    assert_eq!(tags(&mut h, "q"), json!({"a": "2", "B": ""}));
    assert_eq!(
        h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["a", "zzz"]})),
        json!({"ok": true})
    );
    assert_eq!(tags(&mut h, "q"), json!({"B": ""}));
    tag(&mut h, "q", json!({}));
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": []}));
    assert_eq!(tags(&mut h, "q"), json!({"B": ""}));
}

#[test]
fn keys_are_case_sensitive_and_queues_are_independent() {
    let mut h = Harness::new();
    queue(&mut h, "q");
    queue(&mut h, "r");
    tag(&mut h, "q", json!({"k": "1", "K": "2"}));
    assert_eq!(tags(&mut h, "q"), json!({"k": "1", "K": "2"}));
    assert_eq!(tags(&mut h, "r"), json!({}));
}

#[test]
fn value_and_key_validation() {
    let mut h = Harness::new();
    queue(&mut h, "q");
    let t = |h: &mut Harness, v: Value| {
        h.err(json!({"Command": "TagQueue", "QueueName": "q", "Tags": v}))
    };
    // 256 bytes are fine, 257 are not; counted in bytes, not characters.
    tag(&mut h, "q", json!({"k": "x".repeat(256)}));
    tag(&mut h, "q", json!({"k": "é".repeat(128)}));
    assert_eq!(
        t(&mut h, json!({"k": "x".repeat(257)})),
        "InvalidParameterValue"
    );
    assert_eq!(
        t(&mut h, json!({"k": "é".repeat(129)})),
        "InvalidParameterValue"
    );
    assert_eq!(t(&mut h, json!({"k": 1})), "InvalidParameterValue");
    assert_eq!(t(&mut h, json!({"k": null})), "InvalidParameterValue");
    assert_eq!(t(&mut h, json!({"": "v"})), "InvalidParameterValue");
    assert_eq!(t(&mut h, json!({"a b": "v"})), "InvalidParameterValue");
    assert_eq!(
        t(&mut h, json!({"k".repeat(129): "v"})),
        "InvalidParameterValue"
    );
    tag(&mut h, "q", json!({"k".repeat(128): "v"}));
    // A bad member rejects the whole command.
    assert_eq!(
        t(&mut h, json!({"good": "v", "bad key": "v"})),
        "InvalidParameterValue"
    );
    assert_eq!(
        tags(&mut h, "q"),
        json!({"k": "é".repeat(128), "k".repeat(128): "v"})
    );
    assert_eq!(t(&mut h, json!([])), "InvalidParameterValue");
    for bad in [json!([1]), json!(["a b"]), json!([""]), json!({})] {
        assert_eq!(
            h.err(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": bad})),
            "InvalidParameterValue"
        );
    }
}

#[test]
fn missing_parameters_and_unknown_queue() {
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
    // Step 3 precedes the queue lookup; the lookup precedes nothing else here.
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"a b": "v"}})),
        "InvalidParameterValue"
    );
    for cmd in [
        json!({"Command": "TagQueue", "QueueName": "q", "Tags": {}}),
        json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": []}),
        json!({"Command": "ListQueueTags", "QueueName": "q"}),
    ] {
        assert_eq!(h.err(cmd), "QueueDoesNotExist");
    }
}

#[test]
fn at_most_fifty_tags() {
    let mut h = Harness::new();
    queue(&mut h, "q");
    let many = |range: std::ops::Range<u32>| {
        Value::Object(range.map(|i| (format!("k{i}"), json!("v"))).collect())
    };
    tag(&mut h, "q", many(0..50));
    // Replacing existing keys is fine at the limit.
    tag(&mut h, "q", json!({"k0": "new"}));
    let over = json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"extra": "v"}});
    assert_eq!(h.err(over), "LimitExceeded");
    // One new key among replacements still counts.
    let mixed = json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"k1": "x", "extra": "v"}});
    assert_eq!(h.err(mixed), "LimitExceeded");
    // A rejected command changes nothing.
    assert_eq!(tags(&mut h, "q").as_object().unwrap().len(), 50);
    assert_eq!(tags(&mut h, "q")["k1"], "v");
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["k0"]}));
    tag(&mut h, "q", json!({"extra": "v"}));
    // More than 50 in one command on an empty queue.
    queue(&mut h, "r");
    let err = h.err(json!({"Command": "TagQueue", "QueueName": "r", "Tags": many(0..51)}));
    assert_eq!(err, "LimitExceeded");
    assert_eq!(tags(&mut h, "r"), json!({}));
}

#[test]
fn tags_are_not_attributes_and_follow_the_queue_lifecycle() {
    let mut h = Harness::new();
    queue(&mut h, "q");
    tag(&mut h, "q", json!({"a": "1"}));
    let attrs =
        h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}))["Attributes"].clone();
    let mut names: Vec<&String> = attrs.as_object().unwrap().keys().collect();
    names.sort();
    assert_eq!(
        names,
        [
            "ContentBasedDeduplication",
            "DelaySeconds",
            "Fifo",
            "MaxMessageBytes",
            "RetentionSeconds",
            "VisibilityTimeout"
        ]
    );
    // CreateQueue rule 4 ignores tags.
    queue(&mut h, "q");
    h.ok(json!({"Command": "SendMessage", "QueueName": "q", "Body": "x"}));
    h.ok(json!({"Command": "PurgeQueue", "QueueName": "q"}));
    assert_eq!(tags(&mut h, "q"), json!({"a": "1"}));
    h.ok(json!({"Command": "DeleteQueue", "QueueName": "q"}));
    queue(&mut h, "q");
    assert_eq!(tags(&mut h, "q"), json!({}));
}
