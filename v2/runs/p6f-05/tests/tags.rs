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
fn new_queue_has_no_tags_and_responses_have_no_members() {
    let mut h = setup();
    assert_eq!(list(&mut h), json!({}));
    assert_eq!(tag(&mut h, json!({"a": "1"})), json!({"ok": true}));
    let r = h.run(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": []}));
    assert_eq!(r, json!({"ok": true}));
}

#[test]
fn tagging_replaces_values_and_untagging_ignores_unknown_keys() {
    let mut h = setup();
    tag(&mut h, json!({"a": "1", "b": "2", "Empty": ""}));
    tag(&mut h, json!({"a": "3"}));
    assert_eq!(list(&mut h), json!({"a": "3", "b": "2", "Empty": ""}));
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["a", "nope", "A"]}));
    assert_eq!(list(&mut h), json!({"b": "2", "Empty": ""}));
}

#[test]
fn invalid_keys_and_values_are_parameter_errors() {
    let mut h = setup();
    let bad = [
        json!({"": "x"}),
        json!({"a b": "x"}),
        json!({"k".repeat(129): "x"}),
        json!({"a": 1}),
        json!({"a": null}),
        json!({"a": "x".repeat(257)}),
        json!({"a": "\u{e9}".repeat(129)}),
    ];
    for tags in bad {
        assert_eq!(tag(&mut h, tags.clone())["error"]["code"], "InvalidParameterValue", "{tags}");
    }
    // One bad tag rejects the whole command.
    assert_eq!(tag(&mut h, json!({"good": "x", "a b": "x"}))["ok"], false);
    assert_eq!(list(&mut h), json!({}));
    assert_eq!(tag(&mut h, json!({"k".repeat(128): "x".repeat(256)}))["ok"], true);
    assert_eq!(tag(&mut h, json!({"a": "\u{e9}".repeat(128)}))["ok"], true);
}

#[test]
fn untag_rejects_bad_keys() {
    let mut h = setup();
    for keys in [json!(["a b"]), json!([1]), json!([null]), json!("a"), json!({})] {
        let c = json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": keys});
        assert_eq!(h.err(c), "InvalidParameterValue", "{keys}");
    }
}

#[test]
fn check_order_for_tag_commands() {
    let mut h = Harness::new();
    assert_eq!(h.err(json!({"Command": "TagQueue", "QueueName": "q"})), "MissingParameter");
    assert_eq!(h.err(json!({"Command": "TagQueue", "Tags": {}})), "MissingParameter");
    assert_eq!(h.err(json!({"Command": "UntagQueue", "QueueName": "q"})), "MissingParameter");
    assert_eq!(h.err(json!({"Command": "ListQueueTags"})), "MissingParameter");
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "q", "Tags": []})),
        "InvalidParameterValue"
    );
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"a": "b"}})),
        "QueueDoesNotExist"
    );
    assert_eq!(
        h.err(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": []})),
        "QueueDoesNotExist"
    );
    assert_eq!(h.err(json!({"Command": "ListQueueTags", "QueueName": "q"})), "QueueDoesNotExist");
    // A bad value outranks the missing queue.
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "q", "Tags": {"a": 1}})),
        "InvalidParameterValue"
    );
}

#[test]
fn at_most_fifty_tags_and_replacing_does_not_count() {
    let mut h = setup();
    let fifty: serde_json::Map<String, Value> = (0..50).map(|i| (format!("k{i}"), json!("v"))).collect();
    assert_eq!(tag(&mut h, Value::Object(fifty.clone()))["ok"], true);
    assert_eq!(tag(&mut h, json!({"k0": "new"}))["ok"], true);
    assert_eq!(tag(&mut h, json!({"extra": "v"}))["error"]["code"], "LimitExceeded");
    assert_eq!(tag(&mut h, json!({"k1": "x", "extra": "v"}))["error"]["code"], "LimitExceeded");
    let tags = list(&mut h);
    assert_eq!(tags.as_object().unwrap().len(), 50);
    assert_eq!(tags["k0"], "new");
    assert_eq!(tags["k1"], "v");
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["k2"]}));
    assert_eq!(tag(&mut h, json!({"extra": "v"}))["ok"], true);
}

#[test]
fn tags_are_not_attributes_and_survive_purge_but_not_delete() {
    let mut h = setup();
    tag(&mut h, json!({"a": "1"}));
    let attrs = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert!(attrs["Attributes"].get("Tags").is_none());
    assert!(attrs.get("Tags").is_none());
    // CreateQueue rule 4 ignores tags.
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    h.ok(json!({"Command": "SendMessage", "QueueName": "q", "Body": "x"}));
    h.ok(json!({"Command": "PurgeQueue", "QueueName": "q"}));
    assert_eq!(list(&mut h), json!({"a": "1"}));
    h.ok(json!({"Command": "DeleteQueue", "QueueName": "q"}));
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q"}));
    assert_eq!(list(&mut h), json!({}));
}
