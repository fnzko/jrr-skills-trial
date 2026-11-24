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
fn max_message_bytes_is_validated_and_takes_part_in_create_rule_4() {
    let mut h = Harness::new();
    let create_with = |v: serde_json::Value| json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": {"MaxMessageBytes": v}});
    for bad in [json!(0), json!(1025), json!("5"), json!(1.5)] {
        assert_eq!(h.err(create_with(bad)), "InvalidParameterValue");
    }
    h.ok(create_with(json!(1024)));
    h.ok(create_with(json!(null)));
    assert_eq!(h.err(create_with(json!(10))), "QueueAlreadyExists");
    assert_eq!(attrs(&mut h, "q")["MaxMessageBytes"], 1024);
    h.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"MaxMessageBytes": 1}}));
    assert_eq!(attrs(&mut h, "q")["MaxMessageBytes"], 1);
}
