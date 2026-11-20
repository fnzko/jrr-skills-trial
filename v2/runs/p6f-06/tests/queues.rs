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
fn max_message_bytes_is_an_attribute_with_a_range() {
    let mut h = Harness::new();
    let c = |v: serde_json::Value| json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": {"MaxMessageBytes": v}});
    for bad in [json!(0), json!(1025), json!("5"), json!(5.5)] {
        assert_eq!(h.err(c(bad)), "InvalidParameterValue");
    }
    h.ok(c(json!(5)));
    assert_eq!(attrs(&mut h, "q")["MaxMessageBytes"], 5);
    // Part of the identity of an existing queue (CreateQueue rule 4).
    h.ok(c(json!(5)));
    assert_eq!(h.err(c(json!(6))), "QueueAlreadyExists");
    h.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"MaxMessageBytes": 1024}}));
    assert_eq!(attrs(&mut h, "q")["MaxMessageBytes"], 1024);
}

fn tag(h: &mut Harness, tags: serde_json::Value) -> serde_json::Value {
    h.run(json!({"Command": "TagQueue", "QueueName": "q", "Tags": tags}))
}

fn tags(h: &mut Harness) -> serde_json::Value {
    h.ok(json!({"Command": "ListQueueTags", "QueueName": "q"}))["Tags"].clone()
}

#[test]
fn tags_are_set_replaced_and_removed() {
    let mut h = Harness::new();
    create(&mut h, "q");
    assert_eq!(tags(&mut h), json!({}));
    assert_eq!(tag(&mut h, json!({"a": "1", "b": ""}))["ok"], true);
    assert_eq!(tag(&mut h, json!({"a": "2"}))["ok"], true);
    assert_eq!(tags(&mut h), json!({"a": "2", "b": ""}));
    assert_eq!(tag(&mut h, json!({}))["ok"], true);
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["a", "missing"]}));
    assert_eq!(tags(&mut h), json!({"b": ""}));
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": []}));
    assert_eq!(tags(&mut h), json!({"b": ""}));
}

#[test]
fn tag_keys_are_case_sensitive() {
    let mut h = Harness::new();
    create(&mut h, "q");
    tag(&mut h, json!({"Key": "1", "key": "2"}));
    assert_eq!(tags(&mut h), json!({"Key": "1", "key": "2"}));
}

#[test]
fn tag_keys_and_values_are_validated() {
    let mut h = Harness::new();
    create(&mut h, "q");
    let long_key = "k".repeat(129);
    let long_value = "v".repeat(257);
    for bad in [
        json!({"": "v"}),
        json!({"a b": "v"}),
        json!({long_key.clone(): "v"}),
        json!({"a": 1}),
        json!({"a": null}),
        json!({"ok": "v", "a": long_value}),
    ] {
        assert_eq!(tag(&mut h, bad)["error"]["code"], "InvalidParameterValue");
    }
    assert_eq!(tags(&mut h), json!({}));
    let edge = json!({"k".repeat(128): "\u{e9}".repeat(128)});
    assert_eq!(tag(&mut h, edge)["ok"], true);
    assert_eq!(tag(&mut h, json!({"a": "\u{e9}".repeat(129)}))["ok"], false);
    for bad in [json!("a"), json!([1]), json!(["a b"]), json!([""])] {
        let c = json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": bad});
        assert_eq!(h.err(c), "InvalidParameterValue");
    }
    let c = json!({"Command": "TagQueue", "QueueName": "q", "Tags": []});
    assert_eq!(h.err(c), "InvalidParameterValue");
}

#[test]
fn tag_command_check_order() {
    let mut h = Harness::new();
    assert_eq!(
        h.err(json!({"Command": "TagQueue", "QueueName": "q"})),
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
    assert_eq!(tag(&mut h, json!({}))["error"]["code"], "QueueDoesNotExist");
    assert_eq!(
        tag(&mut h, json!({"a b": 1}))["error"]["code"],
        "InvalidParameterValue"
    );
    let c = json!({"Command": "ListQueueTags", "QueueName": "q"});
    assert_eq!(h.err(c), "QueueDoesNotExist");
}

#[test]
fn at_most_fifty_tags_per_queue() {
    let mut h = Harness::new();
    create(&mut h, "q");
    let fifty: serde_json::Map<String, serde_json::Value> =
        (0..50).map(|i| (format!("k{i}"), json!("v"))).collect();
    assert_eq!(tag(&mut h, json!(fifty))["ok"], true);
    assert_eq!(
        tag(&mut h, json!({"extra": "v"}))["error"]["code"],
        "LimitExceeded"
    );
    // Replacing a key the queue has does not count as a new tag.
    assert_eq!(tag(&mut h, json!({"k0": "new"}))["ok"], true);
    h.ok(json!({"Command": "UntagQueue", "QueueName": "q", "TagKeys": ["k1"]}));
    assert_eq!(tag(&mut h, json!({"extra": "v"}))["ok"], true);
    // A failed command changes nothing.
    assert_eq!(
        tag(&mut h, json!({"k1": "a", "k2": "b"}))["error"]["code"],
        "LimitExceeded"
    );
    assert_eq!(tags(&mut h).as_object().unwrap().len(), 50);
    assert_eq!(tags(&mut h)["k0"], "new");
    assert_eq!(tags(&mut h).get("k1"), None);
}

#[test]
fn tags_are_not_attributes_and_survive_purge_but_not_delete() {
    let mut h = Harness::new();
    create(&mut h, "q");
    tag(&mut h, json!({"a": "1"}));
    assert_eq!(attrs(&mut h, "q").get("Tags"), None);
    h.ok(json!({"Command": "PurgeQueue", "QueueName": "q"}));
    assert_eq!(tags(&mut h), json!({"a": "1"}));
    // CreateQueue rule 4 ignores tags.
    create(&mut h, "q");
    h.ok(json!({"Command": "DeleteQueue", "QueueName": "q"}));
    create(&mut h, "q");
    assert_eq!(tags(&mut h), json!({}));
}
