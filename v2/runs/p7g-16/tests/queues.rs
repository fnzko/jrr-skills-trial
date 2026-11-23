mod common;

use common::Session;
use serde_json::json;

fn attrs(s: &mut Session, name: &str) -> serde_json::Value {
    s.ok(json!({"Command": "GetQueueAttributes", "QueueName": name}))["Attributes"].clone()
}

#[test]
fn create_with_defaults_and_get() {
    let mut s = Session::new();
    s.ok(json!({"Command": "CreateQueue", "QueueName": "jobs"}));
    let r = s.ok(json!({"Command": "GetQueueAttributes", "QueueName": "jobs"}));
    assert_eq!(
        r["Attributes"],
        json!({"VisibilityTimeout": 30, "DelaySeconds": 0, "RetentionSeconds": 345600,
               "Fifo": false, "ContentBasedDeduplication": false})
    );
    assert_eq!(
        r["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn create_name_and_attribute_validation() {
    let mut s = Session::new();
    assert_eq!(s.err(json!({"Command": "CreateQueue"})), "MissingParameter");
    assert_eq!(
        s.err(json!({"Command": "CreateQueue", "QueueName": 5})),
        "InvalidParameterValue"
    );
    // The name is checked before the attributes.
    assert_eq!(
        s.err(json!({"Command": "CreateQueue", "QueueName": "a b", "Attributes": 3})),
        "QueueNameInvalid"
    );
    assert_eq!(
        s.err(json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": 3})),
        "InvalidParameterValue"
    );
    assert_eq!(
        s.err(json!({"Command": "CreateQueue", "QueueName": ""})),
        "QueueNameInvalid"
    );
    assert_eq!(
        s.err(json!({"Command": "CreateQueue", "QueueName": "a.fifo"})),
        "QueueNameInvalid"
    );
    assert_eq!(
        s.err(json!({"Command": "CreateQueue", "QueueName": "a", "Attributes": {"Fifo": true}})),
        "QueueNameInvalid"
    );
    for (attr, value) in [
        ("VisibilityTimeout", json!(43201)),
        ("VisibilityTimeout", json!(-1)),
        ("DelaySeconds", json!(901)),
        ("RetentionSeconds", json!(59)),
        ("RetentionSeconds", json!(1209601)),
        ("MaxReceiveCount", json!(0)),
        ("Fifo", json!("true")),
        ("DeadLetterQueue", json!(1)),
        ("Bogus", json!(1)),
    ] {
        let c = json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": {attr: value}});
        assert_eq!(s.err(c), "InvalidParameterValue", "{attr}");
    }
    assert_eq!(
        s.err(json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": {"ContentBasedDeduplication": true}})),
        "InvalidParameterValue"
    );
    assert_eq!(
        s.err(json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": {"MaxReceiveCount": 2}})),
        "InvalidParameterValue"
    );
    // A null attribute is absent.
    s.create("q", json!({"VisibilityTimeout": null, "Bogus": null}));
    assert_eq!(attrs(&mut s, "q")["VisibilityTimeout"], 30);
}

#[test]
fn create_is_idempotent_only_for_equal_attributes() {
    let mut s = Session::new();
    s.create("q", json!({"VisibilityTimeout": 10}));
    s.create("q", json!({"VisibilityTimeout": 10, "DelaySeconds": 0}));
    assert_eq!(
        s.err(json!({"Command": "CreateQueue", "QueueName": "q"})),
        "QueueAlreadyExists"
    );
    assert_eq!(
        s.err(json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": {"VisibilityTimeout": 11}})),
        "QueueAlreadyExists"
    );
}

#[test]
fn create_dead_letter_rules() {
    let mut s = Session::new();
    let create = |dlq: &str, extra: serde_json::Value| {
        let mut attributes = json!({"DeadLetterQueue": dlq, "MaxReceiveCount": 3});
        attributes
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        attributes
    };
    assert_eq!(
        s.err(json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": create("dlq", json!({}))})),
        "QueueDoesNotExist"
    );
    s.create("dlq.fifo", json!({"Fifo": true}));
    assert_eq!(
        s.err(json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": create("dlq.fifo", json!({}))})),
        "InvalidParameterValue"
    );
    s.create("dlq", json!({}));
    s.create("q", create("dlq", json!({})));
    // Same request again is idempotent; a different one is not.
    s.create("q", create("dlq", json!({})));
    assert_eq!(
        s.err(json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": {}})),
        "QueueAlreadyExists"
    );
    let a = attrs(&mut s, "q");
    assert_eq!(a["DeadLetterQueue"], "dlq");
    assert_eq!(a["MaxReceiveCount"], 3);
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
    // An idempotent create of an existing queue still succeeds.
    s.create("q0", json!({}));
}

#[test]
fn set_attributes() {
    let mut s = Session::new();
    s.create("q", json!({}));
    s.create("other", json!({}));
    s.create("f.fifo", json!({"Fifo": true}));
    let set = |name: &str, a: serde_json::Value| json!({"Command": "SetQueueAttributes", "QueueName": name, "Attributes": a});

    assert_eq!(
        s.err(json!({"Command": "SetQueueAttributes", "QueueName": "q"})),
        "MissingParameter"
    );
    assert_eq!(
        s.err(set("nope", json!({"Fifo": true}))),
        "QueueDoesNotExist"
    );
    assert_eq!(
        s.err(set("q", json!({"Fifo": false}))),
        "UnsupportedOperation"
    );
    assert_eq!(
        s.err(set("q", json!({"ContentBasedDeduplication": true}))),
        "InvalidParameterValue"
    );
    assert_eq!(
        s.err(set("q", json!({"MaxReceiveCount": 2}))),
        "InvalidParameterValue"
    );
    assert_eq!(
        s.err(set(
            "q",
            json!({"MaxReceiveCount": 2, "DeadLetterQueue": "zz"})
        )),
        "QueueDoesNotExist"
    );
    assert_eq!(
        s.err(set(
            "q",
            json!({"MaxReceiveCount": 2, "DeadLetterQueue": "q"})
        )),
        "InvalidParameterValue"
    );
    assert_eq!(
        s.err(set(
            "q",
            json!({"MaxReceiveCount": 2, "DeadLetterQueue": "f.fifo"})
        )),
        "InvalidParameterValue"
    );

    s.ok(set("q", json!({})));
    s.ok(set(
        "q",
        json!({"VisibilityTimeout": 5, "MaxReceiveCount": 2, "DeadLetterQueue": "other"}),
    ));
    s.ok(set("f.fifo", json!({"ContentBasedDeduplication": true})));
    let a = attrs(&mut s, "q");
    assert_eq!(a["VisibilityTimeout"], 5);
    assert_eq!(a["DeadLetterQueue"], "other");
    // Replaced but never removed.
    s.ok(set("q", json!({"DelaySeconds": 7})));
    assert_eq!(attrs(&mut s, "q")["DeadLetterQueue"], "other");
}

#[test]
fn list_queues_by_prefix_in_byte_order() {
    let mut s = Session::new();
    for name in ["b", "a_x", "a-x", "A1", "a1", "a.fifo", "ab"] {
        let fifo = name.ends_with(".fifo");
        s.create(name, json!({ "Fifo": fifo }));
    }
    let list = |s: &mut Session, c: serde_json::Value| s.ok(c)["QueueNames"].clone();
    assert_eq!(
        list(&mut s, json!({"Command": "ListQueues"})),
        json!(["A1", "a-x", "a.fifo", "a1", "a_x", "ab", "b"])
    );
    assert_eq!(
        list(&mut s, json!({"Command": "ListQueues", "Prefix": "a"})),
        json!(["a-x", "a.fifo", "a1", "a_x", "ab"])
    );
    assert_eq!(
        list(&mut s, json!({"Command": "ListQueues", "Prefix": "a1"})),
        json!(["a1"])
    );
    assert_eq!(
        list(&mut s, json!({"Command": "ListQueues", "Prefix": "zz"})),
        json!([])
    );
    assert_eq!(
        list(&mut s, json!({"Command": "ListQueues", "Prefix": null}))
            .as_array()
            .unwrap()
            .len(),
        7
    );
    assert_eq!(
        s.err(json!({"Command": "ListQueues", "Prefix": 1})),
        "InvalidParameterValue"
    );
}

#[test]
fn delete_queue_clears_dead_letter_configuration() {
    let mut s = Session::new();
    s.create("dlq", json!({}));
    s.create("q", json!({"DeadLetterQueue": "dlq", "MaxReceiveCount": 2}));
    s.ok(json!({"Command": "DeleteQueue", "QueueName": "dlq"}));
    let a = attrs(&mut s, "q");
    assert!(a.get("DeadLetterQueue").is_none() && a.get("MaxReceiveCount").is_none());
    assert_eq!(
        s.err(json!({"Command": "DeleteQueue", "QueueName": "dlq"})),
        "QueueDoesNotExist"
    );
    assert_eq!(
        s.err(json!({"Command": "GetQueueAttributes", "QueueName": "dlq"})),
        "QueueDoesNotExist"
    );
    // The name can be reused and shares nothing with the old queue.
    s.create("dlq", json!({"VisibilityTimeout": 1}));
    assert!(attrs(&mut s, "q").get("DeadLetterQueue").is_none());
}

#[test]
fn queue_lookup_is_exact_whatever_the_string() {
    let mut s = Session::new();
    for command in ["GetQueueAttributes", "DeleteQueue", "PurgeQueue"] {
        assert_eq!(
            s.err(json!({"Command": command, "QueueName": "no such/queue"})),
            "QueueDoesNotExist"
        );
        assert_eq!(
            s.err(json!({"Command": command, "QueueName": ""})),
            "QueueDoesNotExist"
        );
        assert_eq!(s.err(json!({"Command": command})), "MissingParameter");
        assert_eq!(
            s.err(json!({"Command": command, "QueueName": 4})),
            "InvalidParameterValue"
        );
    }
}
