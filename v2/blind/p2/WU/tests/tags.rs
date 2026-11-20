mod common;

use common::Harness;
use serde_json::{Value, json};

fn queue(h: &mut Harness, name: &str) {
    h.ok(json!({"Command": "CreateQueue", "QueueName": name}));
}

fn tag(h: &mut Harness, name: &str, tags: Value) -> Value {
    h.ok(json!({"Command": "TagQueue", "QueueName": name, "Tags": tags}))
}

fn tags(h: &mut Harness, name: &str) -> Value {
    h.ok(json!({"Command": "ListQueueTags", "QueueName": name}))["Tags"].clone()
}

#[test]
fn tag_list_replace_and_untag() {
    let mut h = Harness::new();
    queue(&mut h, "q");
    assert_eq!(tags(&mut h, "q"), json!({}));
    let response = tag(
        &mut h,
        "q",
        json!({"env": "prod", "team": "a", "empty": ""}),
    );
    assert_eq!(response, json!({"ok": true}));
    tag(&mut h, "q", json!({"env": "dev"}));
    assert_eq!(
        tags(&mut h, "q"),
        json!({"env": "dev", "team": "a", "empty": ""})
    );
    let response = h.ok(json!({"Command": "UntagQueue", "QueueName": "q",
                               "TagKeys": ["env", "missing", "Team"]}));
    assert_eq!(response, json!({"ok": true}));
    assert_eq!(tags(&mut h, "q"), json!({"team": "a", "empty": ""}));
    tag(&mut h, "q", json!({}));
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": []}));
    assert_eq!(tags(&mut h, "q"), json!({"team": "a", "empty": ""}));
}

#[test]
fn tags_are_per_queue_and_not_attributes() {
    let mut h = Harness::new();
    queue(&mut h, "a");
    queue(&mut h, "b");
    tag(&mut h, "a", json!({"k": "v"}));
    assert_eq!(tags(&mut h, "b"), json!({}));
    let attributes =
        h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "a"}))["Attributes"].clone();
    assert_eq!(attributes.as_object().unwrap().len(), 6);
    // CreateQueue rule 4 ignores tags.
    h.ok(json!({"Command": "CreateQueue", "QueueName": "a"}));
    h.ok(json!({"Command": "SetQueueAttributes", "QueueName": "a", "Attributes": {"DelaySeconds": 1}}));
    h.ok(json!({"Command": "PurgeQueue", "QueueName": "a"}));
    assert_eq!(tags(&mut h, "a"), json!({"k": "v"}));
}

#[test]
fn tags_disappear_with_the_queue() {
    let mut h = Harness::new();
    queue(&mut h, "a");
    tag(&mut h, "a", json!({"k": "v"}));
    h.ok(json!({"Command": "DeleteQueue", "QueueName": "a"}));
    queue(&mut h, "a");
    assert_eq!(tags(&mut h, "a"), json!({}));
}

#[test]
fn value_limit_is_256_utf8_bytes() {
    let mut h = Harness::new();
    queue(&mut h, "q");
    let tag_cmd =
        |value: Value| json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"k": value}});
    h.ok(tag_cmd(json!("x".repeat(256))));
    h.ok(tag_cmd(json!("é".repeat(128))));
    assert_eq!(
        h.err(tag_cmd(json!("x".repeat(257)))),
        "InvalidParameterValue"
    );
    assert_eq!(
        h.err(tag_cmd(json!("é".repeat(129)))),
        "InvalidParameterValue"
    );
    assert_eq!(h.err(tag_cmd(json!(5))), "InvalidParameterValue");
    assert_eq!(h.err(tag_cmd(json!(null))), "InvalidParameterValue");
    assert_eq!(tags(&mut h, "q")["k"], "é".repeat(128));
}

#[test]
fn key_rules() {
    let mut h = Harness::new();
    queue(&mut h, "q");
    tag(&mut h, "q", json!({"a-B_9": "1", "K": "2", "k": "3"}));
    assert_eq!(tags(&mut h, "q"), json!({"a-B_9": "1", "K": "2", "k": "3"}));
    tag(&mut h, "q", json!({"x".repeat(128): "v"}));
    for bad in ["", "a b", "é", &"x".repeat(129)] {
        let command = json!({"Command": "TagQueue", "QueueName": "q", "Tags": {bad: "v"}});
        assert_eq!(h.err(command), "InvalidParameterValue", "{bad:?}");
    }
    for bad in [
        json!(""),
        json!("a.b"),
        json!(1),
        json!(null),
        json!("x".repeat(129)),
    ] {
        let command = json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": [bad]});
        assert_eq!(h.err(command), "InvalidParameterValue");
    }
}

#[test]
fn fifty_tags_at_most_and_failure_changes_nothing() {
    let mut h = Harness::new();
    queue(&mut h, "q");
    let many = |range: std::ops::Range<u32>| -> Value {
        range
            .map(|i| (format!("k{i}"), json!("v")))
            .collect::<serde_json::Map<_, _>>()
            .into()
    };
    tag(&mut h, "q", many(0..50));
    // Replacing existing keys is not an addition.
    tag(&mut h, "q", json!({"k0": "new", "k49": "new"}));
    let command =
        json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"k0": "x", "extra": "y"}});
    assert_eq!(h.err(command), "LimitExceeded");
    let listed = tags(&mut h, "q");
    assert_eq!(listed.as_object().unwrap().len(), 50);
    assert_eq!(listed["k0"], "new");
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["k1"]}));
    tag(&mut h, "q", json!({"extra": "y"}));
    let command = json!({"Command": "TagQueue", "QueueName": "q", "Tags": many(100..151)});
    assert_eq!(h.err(command), "LimitExceeded");
}

#[test]
fn check_order() {
    let mut h = Harness::new();
    let cmd = |c: &str| json!({"Command": c});
    assert_eq!(h.err(cmd("TagQueue")), "MissingParameter");
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "nope"})),
        "MissingParameter"
    );
    assert_eq!(
        h.err(json!({"Command": "UntagQueue", "QueueName": "nope"})),
        "MissingParameter"
    );
    assert_eq!(h.err(cmd("ListQueueTags")), "MissingParameter");
    // Value checks (step 3) come before the queue lookup (step 4).
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "nope", "Tags": []})),
        "InvalidParameterValue"
    );
    assert_eq!(
        h.err(json!({"Command": "UntagQueue", "QueueName": "nope", "TagKeys": {}})),
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
    assert_eq!(
        h.err(json!({"Command": "ListQueueTags", "QueueName": "nope"})),
        "QueueDoesNotExist"
    );
}
