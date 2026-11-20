mod common;

use common::Harness;
use serde_json::{Value, json};

fn create(h: &mut Harness, name: &str) {
    h.ok(json!({"Command": "CreateQueue", "QueueName": name}));
}

fn tag(h: &mut Harness, tags: Value) -> Value {
    h.run(json!({"Command": "TagQueue", "QueueName": "q", "Tags": tags}))
}

fn list(h: &mut Harness) -> Value {
    h.ok(json!({"Command": "ListQueueTags", "QueueName": "q"}))
}

#[test]
fn tags_start_empty_and_are_set_replaced_and_removed() {
    let mut h = Harness::new();
    create(&mut h, "q");
    assert_eq!(list(&mut h), json!({"ok": true, "Tags": {}}));
    assert_eq!(tag(&mut h, json!({"a": "1", "B": ""})), json!({"ok": true}));
    assert_eq!(tag(&mut h, json!({"a": "2"})), json!({"ok": true}));
    assert_eq!(list(&mut h)["Tags"], json!({"a": "2", "B": ""}));
    let r = h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["a", "zz"]}));
    assert_eq!(r, json!({"ok": true}));
    assert_eq!(list(&mut h)["Tags"], json!({"B": ""}));
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": []}));
    assert_eq!(tag(&mut h, json!({})), json!({"ok": true}));
}

#[test]
fn tag_values_count_utf8_bytes() {
    let mut h = Harness::new();
    create(&mut h, "q");
    assert_eq!(tag(&mut h, json!({"k": "x".repeat(256)}))["ok"], true);
    assert_eq!(tag(&mut h, json!({"k": "\u{e9}".repeat(128)}))["ok"], true);
    for bad in [
        json!("x".repeat(257)),
        json!("\u{e9}".repeat(129)),
        json!(1),
        json!(null),
    ] {
        assert_eq!(
            tag(&mut h, json!({"k": bad}))["error"]["code"],
            "InvalidParameterValue"
        );
    }
}

#[test]
fn tag_keys_follow_the_identifier_rule() {
    let mut h = Harness::new();
    create(&mut h, "q");
    assert_eq!(tag(&mut h, json!({"k".repeat(128): "v"}))["ok"], true);
    for bad in [
        "".to_owned(),
        "k".repeat(129),
        "a b".to_owned(),
        "\u{e9}".to_owned(),
    ] {
        assert_eq!(
            tag(&mut h, json!({bad.clone(): "v"}))["error"]["code"],
            "InvalidParameterValue"
        );
        let untag = json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": [bad]});
        assert_eq!(h.err(untag), "InvalidParameterValue");
    }
    for bad in [json!(1), json!(null)] {
        let untag = json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": [bad]});
        assert_eq!(h.err(untag), "InvalidParameterValue");
    }
}

#[test]
fn at_most_fifty_tags_and_a_failed_command_changes_nothing() {
    let mut h = Harness::new();
    create(&mut h, "q");
    let fifty: serde_json::Map<String, Value> =
        (0..50).map(|i| (format!("k{i}"), json!("v"))).collect();
    assert_eq!(tag(&mut h, Value::Object(fifty.clone()))["ok"], true);
    assert_eq!(tag(&mut h, json!({"k0": "new"}))["ok"], true);
    assert_eq!(
        tag(&mut h, json!({"extra": "v"}))["error"]["code"],
        "LimitExceeded"
    );
    assert_eq!(list(&mut h)["Tags"].as_object().unwrap().len(), 50);
    assert_eq!(list(&mut h)["Tags"]["k0"], "new");
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["k1"]}));
    assert_eq!(tag(&mut h, json!({"extra": "v", "k0": "x"}))["ok"], true);
}

#[test]
fn check_order_and_parameter_errors() {
    let mut h = Harness::new();
    let missing = |h: &mut Harness, c: Value| h.err(c);
    assert_eq!(
        missing(&mut h, json!({"Command": "TagQueue", "Tags": {}})),
        "MissingParameter"
    );
    assert_eq!(
        missing(&mut h, json!({"Command": "TagQueue", "QueueName": "q"})),
        "MissingParameter"
    );
    assert_eq!(
        missing(&mut h, json!({"Command": "UntagQueue", "QueueName": "q"})),
        "MissingParameter"
    );
    assert_eq!(
        missing(&mut h, json!({"Command": "ListQueueTags"})),
        "MissingParameter"
    );
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "q", "Tags": []})),
        "InvalidParameterValue"
    );
    assert_eq!(
        h.err(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": {}})),
        "InvalidParameterValue"
    );
    assert_eq!(
        tag(&mut h, json!({"k": "v"}))["error"]["code"],
        "QueueDoesNotExist"
    );
    assert_eq!(
        h.err(json!({"Command": "ListQueueTags", "QueueName": "q"})),
        "QueueDoesNotExist"
    );
    create(&mut h, "q");
    assert_eq!(
        tag(&mut h, json!({"bad key": "v"}))["error"]["code"],
        "InvalidParameterValue"
    );
}

#[test]
fn tags_are_not_attributes_and_survive_purge_but_not_delete() {
    let mut h = Harness::new();
    create(&mut h, "q");
    tag(&mut h, json!({"k": "v"}));
    let attrs =
        h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}))["Attributes"].clone();
    assert!(attrs.get("Tags").is_none());
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": {}}));
    h.ok(json!({"Command": "PurgeQueue", "QueueName": "q"}));
    assert_eq!(list(&mut h)["Tags"], json!({"k": "v"}));
    h.ok(json!({"Command": "DeleteQueue", "QueueName": "q"}));
    create(&mut h, "q");
    assert_eq!(list(&mut h)["Tags"], json!({}));
}
