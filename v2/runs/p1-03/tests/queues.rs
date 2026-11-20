mod common;

use common::Session;
use serde_json::json;

fn create(s: &mut Session, name: &str) {
    s.ok(json!({"Command": "CreateQueue", "QueueName": name}));
}

#[test]
fn create_applies_defaults_and_reports_them() {
    let mut s = Session::new();
    create(&mut s, "jobs");
    let r = s.ok(json!({"Command": "GetQueueAttributes", "QueueName": "jobs"}));
    assert_eq!(
        r["Attributes"],
        json!({
            "VisibilityTimeout": 30, "DelaySeconds": 0, "RetentionSeconds": 345600,
            "Fifo": false, "ContentBasedDeduplication": false
        })
    );
    assert_eq!(
        r["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn queue_name_syntax() {
    let mut s = Session::new();
    let long_ok = "a".repeat(80);
    let long_fifo_ok = format!("{}.fifo", "a".repeat(75));
    create(&mut s, &long_ok);
    s.ok(
        json!({"Command": "CreateQueue", "QueueName": long_fifo_ok, "Attributes": {"Fifo": true}}),
    );
    for bad in [
        "",
        "a b",
        "a.b",
        ".fifo",
        "é",
        &"a".repeat(81),
        &format!("{}.fifo", "a".repeat(76)),
        "a.fifo.fifo",
    ] {
        assert_eq!(
            s.err(json!({"Command": "CreateQueue", "QueueName": bad})),
            "QueueNameInvalid",
            "{bad:?}"
        );
    }
}

#[test]
fn name_and_fifo_flag_must_agree() {
    let mut s = Session::new();
    assert_eq!(
        s.err(json!({"Command": "CreateQueue", "QueueName": "q.fifo"})),
        "QueueNameInvalid"
    );
    assert_eq!(
        s.err(json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": {"Fifo": true}})),
        "QueueNameInvalid"
    );
    s.ok(json!({"Command": "CreateQueue", "QueueName": "q.fifo", "Attributes": {"Fifo": true}}));
}

#[test]
fn create_checks_in_documented_order() {
    let mut s = Session::new();
    assert_eq!(s.err(json!({"Command": "CreateQueue"})), "MissingParameter");
    // QueueName is checked before every other parameter.
    assert_eq!(
        s.err(json!({"Command": "CreateQueue", "QueueName": "bad name", "Attributes": 3})),
        "QueueNameInvalid"
    );
    assert_eq!(
        s.err(json!({"Command": "CreateQueue", "QueueName": 3, "Attributes": {"Fifo": 1}})),
        "InvalidParameterValue"
    );
    // Value errors beat the name/Fifo mismatch.
    assert_eq!(
        s.err(json!({"Command": "CreateQueue", "QueueName": "q.fifo", "Attributes": {"DelaySeconds": 901}})),
        "InvalidParameterValue"
    );
    assert_eq!(
        s.err(json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": {"ContentBasedDeduplication": true}})),
        "InvalidParameterValue"
    );
    assert_eq!(
        s.err(json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": {"MaxReceiveCount": 3}})),
        "InvalidParameterValue"
    );
    assert_eq!(
        s.err(json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": {"DeadLetterQueue": "dlq", "MaxReceiveCount": 3}})),
        "QueueDoesNotExist"
    );
}

#[test]
fn attribute_values_are_validated() {
    let mut s = Session::new();
    for attributes in [
        json!({"VisibilityTimeout": 43201}),
        json!({"VisibilityTimeout": -1}),
        json!({"DelaySeconds": "1"}),
        json!({"RetentionSeconds": 59}),
        json!({"RetentionSeconds": 1209601}),
        json!({"MaxReceiveCount": 0, "DeadLetterQueue": "x"}),
        json!({"Fifo": "true"}),
        json!({"Bogus": 1}),
        json!({"Bogus": null}),
    ] {
        assert_eq!(
            s.err(json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": attributes})),
            "InvalidParameterValue",
            "{attributes}"
        );
    }
    assert_eq!(
        s.err(json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": []})),
        "InvalidParameterValue"
    );
    // null means absent.
    s.ok(json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": {"DelaySeconds": null}}));
    s.ok(json!({"Command": "CreateQueue", "QueueName": "q2", "Attributes": null}));
}

#[test]
fn create_is_idempotent_only_for_equal_attributes() {
    let mut s = Session::new();
    s.ok(json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": {"DelaySeconds": 5}}));
    s.ok(json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": {"DelaySeconds": 5, "VisibilityTimeout": 30}}));
    assert_eq!(
        s.err(json!({"Command": "CreateQueue", "QueueName": "q"})),
        "QueueAlreadyExists"
    );
    // The existing queue is judged before the dead-letter queue is looked up.
    assert_eq!(
        s.err(json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": {"DelaySeconds": 5, "DeadLetterQueue": "none", "MaxReceiveCount": 2}})),
        "QueueAlreadyExists"
    );
}

#[test]
fn dead_letter_queue_must_exist_and_match_type() {
    let mut s = Session::new();
    create(&mut s, "dlq");
    s.ok(json!({"Command": "CreateQueue", "QueueName": "f.fifo", "Attributes": {"Fifo": true}}));
    let dead = |name: &str, dlq: &str| {
        json!({"Command": "CreateQueue", "QueueName": name,
               "Attributes": {"DeadLetterQueue": dlq, "MaxReceiveCount": 2, "Fifo": name.ends_with(".fifo")}})
    };
    assert_eq!(s.err(dead("g.fifo", "dlq")), "InvalidParameterValue");
    assert_eq!(s.err(dead("g", "f.fifo")), "InvalidParameterValue");
    s.ok(dead("g", "dlq"));
    s.ok(dead("g", "dlq"));
    let r = s.ok(json!({"Command": "GetQueueAttributes", "QueueName": "g"}));
    assert_eq!(r["Attributes"]["DeadLetterQueue"], "dlq");
    assert_eq!(r["Attributes"]["MaxReceiveCount"], 2);
}

#[test]
fn at_most_1000_queues() {
    let mut s = Session::new();
    for i in 0..1000 {
        create(&mut s, &format!("q{i}"));
    }
    assert_eq!(
        s.err(json!({"Command": "CreateQueue", "QueueName": "extra"})),
        "LimitExceeded"
    );
    // An equal re-create still succeeds at the limit.
    create(&mut s, "q0");
    s.ok(json!({"Command": "DeleteQueue", "QueueName": "q5"}));
    create(&mut s, "extra");
}

#[test]
fn set_attributes_rules_and_effects() {
    let mut s = Session::new();
    create(&mut s, "q");
    create(&mut s, "dlq");
    s.ok(json!({"Command": "CreateQueue", "QueueName": "f.fifo", "Attributes": {"Fifo": true}}));
    let set = |name: &str, attributes| json!({"Command": "SetQueueAttributes", "QueueName": name, "Attributes": attributes});

    assert_eq!(
        s.err(json!({"Command": "SetQueueAttributes", "QueueName": "q"})),
        "MissingParameter"
    );
    assert_eq!(
        s.err(set("nope", json!({"Fifo": 1}))),
        "InvalidParameterValue"
    );
    assert_eq!(s.err(set("nope", json!({}))), "QueueDoesNotExist");
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
            json!({"DeadLetterQueue": "x", "MaxReceiveCount": 2})
        )),
        "QueueDoesNotExist"
    );
    assert_eq!(
        s.err(set(
            "q",
            json!({"DeadLetterQueue": "q", "MaxReceiveCount": 2})
        )),
        "InvalidParameterValue"
    );
    assert_eq!(
        s.err(set(
            "q",
            json!({"DeadLetterQueue": "f.fifo", "MaxReceiveCount": 2})
        )),
        "InvalidParameterValue"
    );

    s.ok(set(
        "q",
        json!({"DelaySeconds": 7, "DeadLetterQueue": "dlq", "MaxReceiveCount": 4}),
    ));
    s.ok(set("q", json!({"VisibilityTimeout": 9})));
    let r = s.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(r["Attributes"]["DelaySeconds"], 7);
    assert_eq!(r["Attributes"]["VisibilityTimeout"], 9);
    assert_eq!(r["Attributes"]["DeadLetterQueue"], "dlq");
    s.ok(set("f.fifo", json!({"ContentBasedDeduplication": true})));
}

#[test]
fn delete_queue_clears_dead_letter_configs_and_forgets_the_name() {
    let mut s = Session::new();
    create(&mut s, "dlq");
    s.ok(json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": {"DeadLetterQueue": "dlq", "MaxReceiveCount": 2}}));
    s.ok(json!({"Command": "DeleteQueue", "QueueName": "dlq"}));
    let r = s.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert!(r["Attributes"].get("DeadLetterQueue").is_none());
    assert!(r["Attributes"].get("MaxReceiveCount").is_none());
    assert_eq!(
        s.err(json!({"Command": "DeleteQueue", "QueueName": "dlq"})),
        "QueueDoesNotExist"
    );
    create(&mut s, "dlq");
    let r = s.ok(json!({"Command": "GetQueueAttributes", "QueueName": "dlq"}));
    assert_eq!(r["Counts"]["Visible"], 0);
}

#[test]
fn list_queues_sorts_by_bytes_and_filters_by_prefix() {
    let mut s = Session::new();
    for name in ["b", "a_", "a-", "A", "a1", "a", "ab", "q.fifo"] {
        let fifo = name.ends_with(".fifo");
        s.ok(json!({"Command": "CreateQueue", "QueueName": name, "Attributes": {"Fifo": fifo}}));
    }
    let r = s.ok(json!({"Command": "ListQueues"}));
    assert_eq!(
        r["QueueNames"],
        json!(["A", "a", "a-", "a1", "a_", "ab", "b", "q.fifo"])
    );
    let r = s.ok(json!({"Command": "ListQueues", "Prefix": "a"}));
    assert_eq!(r["QueueNames"], json!(["a", "a-", "a1", "a_", "ab"]));
    let r = s.ok(json!({"Command": "ListQueues", "Prefix": "zz"}));
    assert_eq!(r["QueueNames"], json!([]));
    assert_eq!(
        s.err(json!({"Command": "ListQueues", "Prefix": 1})),
        "InvalidParameterValue"
    );
    assert_eq!(
        s.ok(json!({"Command": "ListQueues", "Prefix": null}))["QueueNames"]
            .as_array()
            .unwrap()
            .len(),
        8
    );
}

#[test]
fn queue_lookup_is_exact_and_follows_value_checks() {
    let mut s = Session::new();
    for command in ["GetQueueAttributes", "DeleteQueue", "PurgeQueue"] {
        assert_eq!(
            s.err(json!({"Command": command, "QueueName": "!weird"})),
            "QueueDoesNotExist"
        );
        assert_eq!(
            s.err(json!({"Command": command, "QueueName": ""})),
            "QueueDoesNotExist"
        );
        assert_eq!(
            s.err(json!({"Command": command, "QueueName": 5})),
            "InvalidParameterValue"
        );
        assert_eq!(s.err(json!({"Command": command})), "MissingParameter");
    }
}
