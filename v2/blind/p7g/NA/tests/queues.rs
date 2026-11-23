mod common;

use common::Session;
use serde_json::json;

#[test]
fn create_is_idempotent_only_for_identical_attributes() {
    let mut s = Session::new();
    s.create("jobs", json!({}));
    s.create(
        "jobs",
        json!({"VisibilityTimeout": 30, "DelaySeconds": null}),
    );
    assert_eq!(
        s.err(json!({"Command": "CreateQueue", "QueueName": "jobs", "Attributes": {"DelaySeconds": 1}})),
        "QueueAlreadyExists"
    );
}

#[test]
fn create_rule_order() {
    let mut s = Session::new();
    let create = |name: &str, attrs| json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attrs});
    assert_eq!(s.err(create("a.fifo", json!({}))), "QueueNameInvalid");
    assert_eq!(
        s.err(create("a", json!({"Fifo": true}))),
        "QueueNameInvalid"
    );
    assert_eq!(
        s.err(create("a", json!({"ContentBasedDeduplication": true}))),
        "InvalidParameterValue"
    );
    assert_eq!(
        s.err(create("a", json!({"MaxReceiveCount": 3}))),
        "InvalidParameterValue"
    );
    assert_eq!(
        s.err(create(
            "a",
            json!({"DeadLetterQueue": "dlq", "MaxReceiveCount": 3})
        )),
        "QueueDoesNotExist"
    );
    s.create("dlq.fifo", json!({"Fifo": true}));
    assert_eq!(
        s.err(create(
            "a",
            json!({"DeadLetterQueue": "dlq.fifo", "MaxReceiveCount": 3})
        )),
        "InvalidParameterValue"
    );
    s.create(
        "b.fifo",
        json!({"Fifo": true, "DeadLetterQueue": "dlq.fifo", "MaxReceiveCount": 3}),
    );
    // An existing queue is compared before the dead-letter queue is looked up.
    assert_eq!(
        s.err(create(
            "b.fifo",
            json!({"Fifo": true, "DeadLetterQueue": "nope.fifo", "MaxReceiveCount": 3})
        )),
        "QueueAlreadyExists"
    );
}

#[test]
fn queue_limit_is_1000() {
    let mut s = Session::new();
    for i in 0..1000 {
        s.create(&format!("q{i}"), json!({}));
    }
    assert_eq!(
        s.err(json!({"Command": "CreateQueue", "QueueName": "extra"})),
        "LimitExceeded"
    );
    // Re-creating an existing queue does not count against the limit.
    s.create("q0", json!({}));
}

#[test]
fn lookup_comes_before_command_rules() {
    let mut s = Session::new();
    assert_eq!(
        s.err(json!({"Command": "PurgeQueue", "QueueName": "not a name"})),
        "QueueDoesNotExist"
    );
    assert_eq!(
        s.err(json!({"Command": "GetQueueAttributes", "QueueName": ""})),
        "QueueDoesNotExist"
    );
    assert_eq!(
        s.err(
            json!({"Command": "SetQueueAttributes", "QueueName": "x", "Attributes": {"Fifo": true}})
        ),
        "QueueDoesNotExist"
    );
    assert_eq!(
        s.err(json!({"Command": "PurgeQueue", "QueueName": 4})),
        "InvalidParameterValue"
    );
}

#[test]
fn set_attributes_rules_and_effects() {
    let mut s = Session::new();
    s.create("a", json!({}));
    s.create("b", json!({}));
    s.create("f.fifo", json!({"Fifo": true}));
    let set = |q: &str, attrs| json!({"Command": "SetQueueAttributes", "QueueName": q, "Attributes": attrs});
    assert_eq!(
        s.err(set("a", json!({"Fifo": false}))),
        "UnsupportedOperation"
    );
    assert_eq!(
        s.err(set("a", json!({"ContentBasedDeduplication": true}))),
        "InvalidParameterValue"
    );
    assert_eq!(
        s.err(set("a", json!({"DeadLetterQueue": "b"}))),
        "InvalidParameterValue"
    );
    assert_eq!(
        s.err(set(
            "a",
            json!({"DeadLetterQueue": "zz", "MaxReceiveCount": 2})
        )),
        "QueueDoesNotExist"
    );
    assert_eq!(
        s.err(set(
            "a",
            json!({"DeadLetterQueue": "a", "MaxReceiveCount": 2})
        )),
        "InvalidParameterValue"
    );
    assert_eq!(
        s.err(set(
            "a",
            json!({"DeadLetterQueue": "f.fifo", "MaxReceiveCount": 2})
        )),
        "InvalidParameterValue"
    );
    s.ok(set(
        "a",
        json!({"DeadLetterQueue": "b", "MaxReceiveCount": 2, "DelaySeconds": 5}),
    ));
    // Other attributes keep their values; the dead-letter pair is reported.
    let r = s.ok(json!({"Command": "GetQueueAttributes", "QueueName": "a"}));
    assert_eq!(
        r["Attributes"],
        json!({
            "VisibilityTimeout": 30, "DelaySeconds": 5, "RetentionSeconds": 345600,
            "Fifo": false, "ContentBasedDeduplication": false,
            "DeadLetterQueue": "b", "MaxReceiveCount": 2
        })
    );
    // An empty change is fine, and the configuration cannot be removed.
    s.ok(set("a", json!({})));
    s.ok(set("a", json!({"DeadLetterQueue": null})));
    let r = s.ok(json!({"Command": "GetQueueAttributes", "QueueName": "b"}));
    assert!(r["Attributes"].get("DeadLetterQueue").is_none());
}

#[test]
fn deleting_a_queue_detaches_dead_letter_configs() {
    let mut s = Session::new();
    s.create("dlq", json!({}));
    s.create("a", json!({"DeadLetterQueue": "dlq", "MaxReceiveCount": 1}));
    s.ok(json!({"Command": "DeleteQueue", "QueueName": "dlq"}));
    let r = s.ok(json!({"Command": "GetQueueAttributes", "QueueName": "a"}));
    assert!(r["Attributes"].get("DeadLetterQueue").is_none());
    assert!(r["Attributes"].get("MaxReceiveCount").is_none());
    assert_eq!(
        s.err(json!({"Command": "DeleteQueue", "QueueName": "dlq"})),
        "QueueDoesNotExist"
    );
    // The same name can be reused, and starts empty.
    s.create("dlq", json!({}));
    assert_eq!(s.counts("dlq"), [0, 0, 0]);
}

#[test]
fn list_queues_sorts_by_byte_and_filters_by_prefix() {
    let mut s = Session::new();
    for name in ["b", "a_x", "a-x", "a.fifo", "a1", "aB", "A"] {
        s.create(name, json!({"Fifo": name.ends_with(".fifo")}));
    }
    let r = s.ok(json!({"Command": "ListQueues"}));
    assert_eq!(
        r["QueueNames"],
        json!(["A", "a-x", "a.fifo", "a1", "aB", "a_x", "b"])
    );
    let r = s.ok(json!({"Command": "ListQueues", "Prefix": "a"}));
    assert_eq!(r["QueueNames"], json!(["a-x", "a.fifo", "a1", "aB", "a_x"]));
    let r = s.ok(json!({"Command": "ListQueues", "Prefix": null}));
    assert_eq!(r["QueueNames"].as_array().unwrap().len(), 7);
    let r = s.ok(json!({"Command": "ListQueues", "Prefix": "zzz"}));
    assert_eq!(r["QueueNames"], json!([]));
}

#[test]
fn tick_reports_the_new_clock() {
    let mut s = Session::new();
    assert_eq!(s.ok(json!({"Command": "Tick", "Seconds": 5}))["Now"], 5);
    assert_eq!(s.ok(json!({"Command": "Tick", "Seconds": 0}))["Now"], 5);
    assert_eq!(
        s.err(json!({"Command": "Tick", "Seconds": 1209601})),
        "InvalidParameterValue"
    );
    assert_eq!(s.err(json!({"Command": "Tick"})), "MissingParameter");
}
