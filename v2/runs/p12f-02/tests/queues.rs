mod common;

use common::Harness;
use serde_json::json;

fn create(h: &mut Harness, name: &str) {
    h.ok(json!({"Command": "CreateQueue", "QueueName": name}));
}

fn create_fifo(h: &mut Harness, name: &str) {
    h.ok(json!({"Command": "CreateQueue", "QueueName": name, "Attributes": {"Fifo": true}}));
}

fn attrs(h: &mut Harness, name: &str) -> serde_json::Value {
    h.ok(json!({"Command": "GetQueueAttributes", "QueueName": name}))["Attributes"].clone()
}

#[test]
fn create_uses_defaults() {
    let mut h = Harness::new();
    create(&mut h, "jobs");
    let r = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "jobs"}));
    assert_eq!(
        r["Attributes"],
        json!({
            "VisibilityTimeout": 30, "DelaySeconds": 0, "RetentionSeconds": 345600,
            "Fifo": false, "ContentBasedDeduplication": false, "MaxMessageBytes": 1024,
        })
    );
    assert_eq!(
        r["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn create_checks_name_before_everything_else() {
    let mut h = Harness::new();
    let c = |name: serde_json::Value, attrs: serde_json::Value| json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attrs});
    assert_eq!(h.err(json!({"Command": "CreateQueue"})), "MissingParameter");
    assert_eq!(h.err(c(json!(5), json!(5))), "InvalidParameterValue");
    assert_eq!(h.err(c(json!("bad name"), json!(5))), "QueueNameInvalid");
    assert_eq!(h.err(c(json!(""), json!({}))), "QueueNameInvalid");
    assert_eq!(h.err(c(json!("q"), json!(5))), "InvalidParameterValue");
    assert_eq!(
        h.err(c(json!("q"), json!({"Nope": 1}))),
        "InvalidParameterValue"
    );
    assert_eq!(
        h.err(c(json!("q"), json!({"VisibilityTimeout": 43201}))),
        "InvalidParameterValue"
    );
    assert_eq!(
        h.err(c(json!("q"), json!({"RetentionSeconds": 59}))),
        "InvalidParameterValue"
    );
    assert_eq!(
        h.err(c(json!("q"), json!({"MaxReceiveCount": 0}))),
        "InvalidParameterValue"
    );
    assert_eq!(
        h.err(c(json!("q"), json!({"Fifo": "yes"}))),
        "InvalidParameterValue"
    );
    assert_eq!(
        h.err(c(json!("q"), json!({"Fifo": true}))),
        "QueueNameInvalid"
    );
    assert_eq!(h.err(c(json!("q.fifo"), json!({}))), "QueueNameInvalid");
    assert_eq!(
        h.err(c(json!("q"), json!({"ContentBasedDeduplication": true}))),
        "InvalidParameterValue"
    );
    assert_eq!(
        h.err(c(json!("q"), json!({"MaxReceiveCount": 3}))),
        "InvalidParameterValue"
    );
    assert_eq!(
        h.err(c(json!("q"), json!({"DeadLetterQueue": "x"}))),
        "InvalidParameterValue"
    );
    assert_eq!(
        h.err(c(
            json!("q"),
            json!({"DeadLetterQueue": "x", "MaxReceiveCount": 3})
        )),
        "QueueDoesNotExist"
    );
    // Nothing above created a queue.
    assert_eq!(
        h.ok(json!({"Command": "ListQueues"}))["QueueNames"],
        json!([])
    );
}

#[test]
fn null_attributes_count_as_absent() {
    let mut h = Harness::new();
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": null}));
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q",
                "Attributes": {"VisibilityTimeout": null, "DeadLetterQueue": null}}));
}

#[test]
fn create_is_idempotent_only_for_equal_attributes() {
    let mut h = Harness::new();
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": {"DelaySeconds": 5}}));
    h.ok(json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": {"DelaySeconds": 5}}));
    assert_eq!(
        h.err(json!({"Command": "CreateQueue", "QueueName": "q"})),
        "QueueAlreadyExists"
    );
    // Equality is checked before the dead-letter queue is looked up.
    assert_eq!(
        h.err(json!({"Command": "CreateQueue", "QueueName": "q",
                     "Attributes": {"DeadLetterQueue": "nope", "MaxReceiveCount": 2}})),
        "QueueAlreadyExists"
    );
}

#[test]
fn dead_letter_configuration_rules() {
    let mut h = Harness::new();
    create(&mut h, "dlq");
    create_fifo(&mut h, "dlq.fifo");
    let dl = |name: &str, target: &str| {
        json!({"Command": "CreateQueue", "QueueName": name,
               "Attributes": {"DeadLetterQueue": target, "MaxReceiveCount": 2}})
    };
    h.ok(dl("a", "dlq"));
    assert_eq!(h.err(dl("b", "dlq.fifo")), "InvalidParameterValue");
    assert_eq!(h.err(dl("b", "missing")), "QueueDoesNotExist");
    // An existing queue with the same configuration is accepted again.
    h.ok(dl("a", "dlq"));
    assert_eq!(attrs(&mut h, "a")["DeadLetterQueue"], "dlq");
    assert_eq!(attrs(&mut h, "a")["MaxReceiveCount"], 2);
}

#[test]
fn queue_limit_is_1000() {
    let mut h = Harness::new();
    for i in 0..1000 {
        create(&mut h, &format!("q{i}"));
    }
    assert_eq!(
        h.err(json!({"Command": "CreateQueue", "QueueName": "extra"})),
        "LimitExceeded"
    );
    // Re-creating an existing queue still works at the limit.
    create(&mut h, "q0");
    h.ok(json!({"Command": "DeleteQueue", "QueueName": "q0"}));
    create(&mut h, "extra");
}

#[test]
fn set_attributes_rules() {
    let mut h = Harness::new();
    create(&mut h, "q");
    create(&mut h, "dlq");
    create_fifo(&mut h, "f.fifo");
    let set = |name: &str, a: serde_json::Value| json!({"Command": "SetQueueAttributes", "QueueName": name, "Attributes": a});
    assert_eq!(
        h.err(json!({"Command": "SetQueueAttributes", "QueueName": "q"})),
        "MissingParameter"
    );
    assert_eq!(
        h.err(set("nope", json!({"Fifo": 7}))),
        "InvalidParameterValue"
    );
    assert_eq!(h.err(set("nope", json!({}))), "QueueDoesNotExist");
    assert_eq!(
        h.err(set("q", json!({"Fifo": false}))),
        "UnsupportedOperation"
    );
    assert_eq!(
        h.err(set("q", json!({"ContentBasedDeduplication": true}))),
        "InvalidParameterValue"
    );
    assert_eq!(
        h.err(set("q", json!({"MaxReceiveCount": 2}))),
        "InvalidParameterValue"
    );
    assert_eq!(
        h.err(set(
            "q",
            json!({"DeadLetterQueue": "x", "MaxReceiveCount": 2})
        )),
        "QueueDoesNotExist"
    );
    assert_eq!(
        h.err(set(
            "q",
            json!({"DeadLetterQueue": "q", "MaxReceiveCount": 2})
        )),
        "InvalidParameterValue"
    );
    assert_eq!(
        h.err(set(
            "q",
            json!({"DeadLetterQueue": "f.fifo", "MaxReceiveCount": 2})
        )),
        "InvalidParameterValue"
    );
    // A failed command changes nothing, even when other attributes are valid.
    assert_eq!(
        h.err(set("q", json!({"DelaySeconds": 9, "Fifo": true}))),
        "UnsupportedOperation"
    );
    assert_eq!(attrs(&mut h, "q")["DelaySeconds"], 0);

    h.ok(set(
        "q",
        json!({"DelaySeconds": 9, "DeadLetterQueue": "dlq", "MaxReceiveCount": 4}),
    ));
    h.ok(set("q", json!({})));
    let a = attrs(&mut h, "q");
    assert_eq!(
        (a["DelaySeconds"].clone(), a["VisibilityTimeout"].clone()),
        (json!(9), json!(30))
    );
    assert_eq!(a["DeadLetterQueue"], "dlq");
    h.ok(set("f.fifo", json!({"ContentBasedDeduplication": true})));
}

#[test]
fn list_queues_sorts_by_bytes_and_filters_by_prefix() {
    let mut h = Harness::new();
    for name in ["b", "a_", "a-", "A", "a1", "a", "a.fifo"] {
        if name.ends_with(".fifo") {
            h.ok(
                json!({"Command": "CreateQueue", "QueueName": name, "Attributes": {"Fifo": true}}),
            );
        } else {
            create(&mut h, name);
        }
    }
    let all = h.ok(json!({"Command": "ListQueues"}))["QueueNames"].clone();
    assert_eq!(all, json!(["A", "a", "a-", "a.fifo", "a1", "a_", "b"]));
    let a = h.ok(json!({"Command": "ListQueues", "Prefix": "a"}))["QueueNames"].clone();
    assert_eq!(a, json!(["a", "a-", "a.fifo", "a1", "a_"]));
    assert_eq!(
        h.err(json!({"Command": "ListQueues", "Prefix": 1})),
        "InvalidParameterValue"
    );
    let none = h.ok(json!({"Command": "ListQueues", "Prefix": "zz"}))["QueueNames"].clone();
    assert_eq!(none, json!([]));
}

#[test]
fn delete_queue_unsets_dead_letter_configuration_of_others() {
    let mut h = Harness::new();
    create(&mut h, "dlq");
    h.ok(json!({"Command": "CreateQueue", "QueueName": "a",
                "Attributes": {"DeadLetterQueue": "dlq", "MaxReceiveCount": 2}}));
    h.ok(json!({"Command": "DeleteQueue", "QueueName": "dlq"}));
    let a = attrs(&mut h, "a");
    assert!(a.get("DeadLetterQueue").is_none() && a.get("MaxReceiveCount").is_none());
    assert_eq!(
        h.err(json!({"Command": "DeleteQueue", "QueueName": "dlq"})),
        "QueueDoesNotExist"
    );
    assert_eq!(
        h.err(json!({"Command": "GetQueueAttributes", "QueueName": "dlq"})),
        "QueueDoesNotExist"
    );
    // The name is free again and the new queue is a fresh one.
    h.ok(json!({"Command": "CreateQueue", "QueueName": "dlq", "Attributes": {"DelaySeconds": 1}}));
}

#[test]
fn queue_lookup_is_exact() {
    let mut h = Harness::new();
    create(&mut h, "q");
    for name in ["Q", "q ", "", "q.fifo"] {
        for cmd in ["GetQueueAttributes", "PurgeQueue", "DeleteQueue"] {
            assert_eq!(
                h.err(json!({"Command": cmd, "QueueName": name})),
                "QueueDoesNotExist"
            );
        }
    }
    assert_eq!(h.err(json!({"Command": "PurgeQueue"})), "MissingParameter");
    assert_eq!(
        h.err(json!({"Command": "PurgeQueue", "QueueName": 3})),
        "InvalidParameterValue"
    );
}

#[test]
fn max_message_bytes_is_an_attribute_with_range_and_idempotent_create() {
    let mut h = Harness::new();
    let attrs = |v: serde_json::Value| json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": {"MaxMessageBytes": v}});
    assert_eq!(h.err(attrs(json!(0))), "InvalidParameterValue");
    assert_eq!(h.err(attrs(json!(1025))), "InvalidParameterValue");
    assert_eq!(h.err(attrs(json!("5"))), "InvalidParameterValue");
    h.ok(attrs(json!(10)));
    h.ok(attrs(json!(10)));
    assert_eq!(h.err(attrs(json!(11))), "QueueAlreadyExists");
    h.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"MaxMessageBytes": 1}}));
    let r = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(r["Attributes"]["MaxMessageBytes"], 1);
}

fn tag_cmd(
    command: &str,
    queue: &str,
    member: &str,
    value: serde_json::Value,
) -> serde_json::Value {
    json!({"Command": command, "QueueName": queue, member: value})
}

fn tags_of(h: &mut Harness, queue: &str) -> serde_json::Value {
    h.ok(json!({"Command": "ListQueueTags", "QueueName": queue}))["Tags"].clone()
}

#[test]
fn tags_start_empty_and_are_set_replaced_and_removed() {
    let mut h = Harness::new();
    create(&mut h, "q");
    assert_eq!(tags_of(&mut h, "q"), json!({}));
    let r = h.ok(tag_cmd("TagQueue", "q", "Tags", json!({"a": "1", "B": ""})));
    assert_eq!(r, json!({"ok": true}));
    h.ok(tag_cmd("TagQueue", "q", "Tags", json!({"a": "2"})));
    assert_eq!(tags_of(&mut h, "q"), json!({"a": "2", "B": ""}));
    h.ok(tag_cmd("UntagQueue", "q", "TagKeys", json!(["a", "zz"])));
    assert_eq!(tags_of(&mut h, "q"), json!({"B": ""}));
    h.ok(tag_cmd("TagQueue", "q", "Tags", json!({})));
    h.ok(tag_cmd("UntagQueue", "q", "TagKeys", json!([])));
    assert_eq!(tags_of(&mut h, "q"), json!({"B": ""}));
}

#[test]
fn tag_values_are_checked_before_the_queue_lookup() {
    let mut h = Harness::new();
    create(&mut h, "q");
    let long = "x".repeat(257);
    let two_byte_128 = "\u{e9}".repeat(128);
    let two_byte_129 = "\u{e9}".repeat(129);
    for (tags, code) in [
        (json!({"bad key": "v"}), "InvalidParameterValue"),
        (json!({"": "v"}), "InvalidParameterValue"),
        (json!({"k".repeat(129): "v"}), "InvalidParameterValue"),
        (json!({"k": 5}), "InvalidParameterValue"),
        (json!({"k": null}), "InvalidParameterValue"),
        (json!({"k": long}), "InvalidParameterValue"),
        (json!({"k": two_byte_129}), "InvalidParameterValue"),
        (json!([]), "InvalidParameterValue"),
    ] {
        assert_eq!(
            h.err(tag_cmd("TagQueue", "nope", "Tags", tags.clone())),
            code,
            "{tags}"
        );
        assert_eq!(
            h.err(tag_cmd("TagQueue", "q", "Tags", tags.clone())),
            code,
            "{tags}"
        );
    }
    h.ok(tag_cmd(
        "TagQueue",
        "q",
        "Tags",
        json!({"k".repeat(128): "x".repeat(256), "e": two_byte_128}),
    ));
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "q"})),
        "MissingParameter"
    );
    assert_eq!(
        h.err(tag_cmd("TagQueue", "nope", "Tags", json!({}))),
        "QueueDoesNotExist"
    );
    for keys in [json!([1]), json!(["a b"]), json!("a"), json!([null])] {
        assert_eq!(
            h.err(tag_cmd("UntagQueue", "q", "TagKeys", keys)),
            "InvalidParameterValue"
        );
    }
    assert_eq!(
        h.err(tag_cmd("UntagQueue", "nope", "TagKeys", json!([]))),
        "QueueDoesNotExist"
    );
    assert_eq!(
        h.err(json!({"Command": "ListQueueTags", "QueueName": "nope"})),
        "QueueDoesNotExist"
    );
}

#[test]
fn fifty_tags_fit_and_a_failed_tag_command_changes_nothing() {
    let mut h = Harness::new();
    create(&mut h, "q");
    let many = |range: std::ops::Range<u32>| {
        serde_json::Value::Object(range.map(|i| (format!("k{i}"), json!("v"))).collect())
    };
    h.ok(tag_cmd("TagQueue", "q", "Tags", many(0..50)));
    assert_eq!(
        h.err(tag_cmd("TagQueue", "q", "Tags", many(49..51))),
        "LimitExceeded"
    );
    assert_eq!(tags_of(&mut h, "q"), many(0..50));
    // Replacing existing keys adds nothing.
    h.ok(tag_cmd("TagQueue", "q", "Tags", json!({"k0": "new"})));
    h.ok(tag_cmd("UntagQueue", "q", "TagKeys", json!(["k0"])));
    h.ok(tag_cmd("TagQueue", "q", "Tags", json!({"fresh": "v"})));
    assert_eq!(tags_of(&mut h, "q").as_object().unwrap().len(), 50);
}

#[test]
fn tags_are_not_attributes_and_follow_the_queue_lifecycle() {
    let mut h = Harness::new();
    create(&mut h, "q");
    h.ok(tag_cmd("TagQueue", "q", "Tags", json!({"a": "1"})));
    // Tags do not take part in CreateQueue rule 4.
    create(&mut h, "q");
    let attrs = h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(attrs["Attributes"].get("Tags"), None);
    h.ok(json!({"Command": "SendMessage", "QueueName": "q", "Body": "b"}));
    h.ok(json!({"Command": "PurgeQueue", "QueueName": "q"}));
    assert_eq!(tags_of(&mut h, "q"), json!({"a": "1"}));
    h.ok(json!({"Command": "DeleteQueue", "QueueName": "q"}));
    create(&mut h, "q");
    assert_eq!(tags_of(&mut h, "q"), json!({}));
}
