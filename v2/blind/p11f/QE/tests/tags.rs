mod common;

use common::Harness;
use serde_json::{Value, json};

fn create(h: &mut Harness, name: &str) {
    h.ok(json!({"Command": "CreateQueue", "QueueName": name}));
}

fn tag(h: &mut Harness, queue: &str, tags: Value) {
    h.ok(json!({"Command": "TagQueue", "QueueName": queue, "Tags": tags}));
}

fn tags_of(h: &mut Harness, queue: &str) -> Value {
    h.ok(json!({"Command": "ListQueueTags", "QueueName": queue}))["Tags"].clone()
}

#[test]
fn new_queue_has_no_tags() {
    let mut h = Harness::new();
    create(&mut h, "q");
    assert_eq!(
        h.ok(json!({"Command": "ListQueueTags", "QueueName": "q"})),
        json!({"ok": true, "Tags": {}})
    );
}

#[test]
fn tag_sets_and_replaces_values() {
    let mut h = Harness::new();
    create(&mut h, "q");
    assert_eq!(
        h.ok(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"a": "1", "B": ""}})),
        json!({"ok": true})
    );
    tag(&mut h, "q", json!({"a": "2", "b": "x"}));
    tag(&mut h, "q", json!({}));
    assert_eq!(tags_of(&mut h, "q"), json!({"a": "2", "B": "", "b": "x"}));
}

#[test]
fn untag_removes_given_keys_and_ignores_unknown_ones() {
    let mut h = Harness::new();
    create(&mut h, "q");
    tag(&mut h, "q", json!({"a": "1", "b": "2", "c": "3"}));
    assert_eq!(
        h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["a", "zzz", "A"]})),
        json!({"ok": true})
    );
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": []}));
    assert_eq!(tags_of(&mut h, "q"), json!({"b": "2", "c": "3"}));
}

#[test]
fn tag_validation_errors() {
    let mut h = Harness::new();
    create(&mut h, "q");
    let tag_cmd = |tags: Value| json!({"Command": "TagQueue", "QueueName": "q", "Tags": tags});
    let invalid = [
        json!({"": "v"}),
        json!({"a b": "v"}),
        json!({"é": "v"}),
        json!({"k".repeat(129): "v"}),
        json!({"a": 1}),
        json!({"a": ["x"]}),
        json!({"a": "é".repeat(129)}),
        json!({"a": "x".repeat(257)}),
        json!("not an object"),
        json!([]),
    ];
    for tags in invalid {
        assert_eq!(
            h.err(tag_cmd(tags.clone())),
            "InvalidParameterValue",
            "{tags}"
        );
    }
    tag(&mut h, "q", json!({"k".repeat(128): "x".repeat(256)}));
    tag(&mut h, "q", json!({"a": "é".repeat(128)}));
    let untag_invalid = [
        json!("a"),
        json!([1]),
        json!([""]),
        json!(["a b"]),
        json!([null]),
    ];
    for keys in untag_invalid {
        let cmd = json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": keys});
        assert_eq!(h.err(cmd), "InvalidParameterValue", "{keys}");
    }
    assert_eq!(tags_of(&mut h, "q").as_object().unwrap().len(), 2);
}

#[test]
fn invalid_tag_in_a_request_changes_nothing() {
    let mut h = Harness::new();
    create(&mut h, "q");
    let cmd = json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"ok": "1", "bad key": "2"}});
    assert_eq!(h.err(cmd), "InvalidParameterValue");
    assert_eq!(tags_of(&mut h, "q"), json!({}));
}

#[test]
fn check_order_for_tag_commands() {
    let mut h = Harness::new();
    for command in ["TagQueue", "UntagQueue", "ListQueueTags"] {
        assert_eq!(h.err(json!({"Command": command})), "MissingParameter");
        assert_eq!(
            h.err(json!({"Command": command, "QueueName": "nope", "Tags": {}, "TagKeys": []})),
            "QueueDoesNotExist"
        );
    }
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "nope"})),
        "MissingParameter"
    );
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "nope", "Tags": {"a b": "1"}})),
        "InvalidParameterValue"
    );
    assert_eq!(
        h.err(json!({"Command": "UntagQueue", "QueueName": "nope", "TagKeys": null})),
        "MissingParameter"
    );
    assert_eq!(
        h.err(json!({"Command": "UntagQueue", "QueueName": "nope", "TagKeys": [5]})),
        "InvalidParameterValue"
    );
}

#[test]
fn at_most_fifty_tags() {
    let mut h = Harness::new();
    create(&mut h, "q");
    let many = |range: std::ops::Range<u32>| -> Value {
        range
            .map(|i| (format!("k{i}"), json!("v")))
            .collect::<serde_json::Map<_, _>>()
            .into()
    };
    tag(&mut h, "q", many(0..50));
    // Replacing existing keys and an empty request stay within the limit.
    tag(&mut h, "q", json!({"k0": "new"}));
    tag(&mut h, "q", json!({}));
    let over = json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"k0": "x", "extra": "y"}});
    assert_eq!(h.err(over), "LimitExceeded");
    let tags = tags_of(&mut h, "q");
    assert_eq!(tags.as_object().unwrap().len(), 50);
    assert_eq!(tags["k0"], "new");
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["k1"]}));
    tag(&mut h, "q", json!({"extra": "y"}));
    let too_many = json!({"Command": "TagQueue", "QueueName": "q", "Tags": many(100..151)});
    assert_eq!(h.err(too_many), "LimitExceeded");
}

#[test]
fn tags_are_not_attributes_and_survive_purge_but_not_delete() {
    let mut h = Harness::new();
    create(&mut h, "q");
    tag(&mut h, "q", json!({"a": "1"}));
    let attrs = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(attrs["Attributes"].get("Tags"), None);
    assert_eq!(attrs["Attributes"].get("a"), None);
    // CreateQueue equality ignores tags.
    create(&mut h, "q");
    assert_eq!(tags_of(&mut h, "q"), json!({"a": "1"}));
    h.ok(json!({"Command": "PurgeQueue", "QueueName": "q"}));
    assert_eq!(tags_of(&mut h, "q"), json!({"a": "1"}));
    h.ok(json!({"Command": "DeleteQueue", "QueueName": "q"}));
    create(&mut h, "q");
    assert_eq!(tags_of(&mut h, "q"), json!({}));
}

#[test]
fn tags_belong_to_one_queue() {
    let mut h = Harness::new();
    create(&mut h, "a");
    create(&mut h, "b");
    tag(&mut h, "a", json!({"k": "v"}));
    assert_eq!(tags_of(&mut h, "b"), json!({}));
}
