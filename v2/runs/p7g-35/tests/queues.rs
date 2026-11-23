mod common;

use common::Session;
use serde_json::{json, Value};

fn create(s: &mut Session, name: &str, attributes: Value) {
    s.ok(json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attributes}));
}

fn attributes(s: &mut Session, name: &str) -> Value {
    s.ok(json!({"Command": "GetQueueAttributes", "QueueName": name}))["Attributes"].clone()
}

fn names(s: &mut Session, request: Value) -> Value {
    s.ok(request)["QueueNames"].clone()
}

#[test]
fn a_new_queue_has_the_default_attributes_and_no_messages() {
    let mut s = Session::new();
    s.ok(json!({"Command": "CreateQueue", "QueueName": "jobs"}));
    let r = s.ok(json!({"Command": "GetQueueAttributes", "QueueName": "jobs"}));
    assert_eq!(
        r["Attributes"],
        json!({
            "VisibilityTimeout": 30,
            "DelaySeconds": 0,
            "RetentionSeconds": 345600,
            "Fifo": false,
            "ContentBasedDeduplication": false,
        })
    );
    assert_eq!(
        r["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn queue_names_follow_the_naming_rules() {
    let mut s = Session::new();
    let long = "a".repeat(80);
    let too_long = "a".repeat(81);
    let fifo_max = format!("{}.fifo", "a".repeat(75));
    let fifo_too_long = format!("{}.fifo", "a".repeat(76));
    for name in ["a", "A_b-9", long.as_str()] {
        s.ok(json!({"Command": "CreateQueue", "QueueName": name}));
    }
    s.ok(json!({"Command": "CreateQueue", "QueueName": fifo_max, "Attributes": {"Fifo": true}}));
    for name in [
        "",
        "a b",
        "a.b",
        "a/b",
        "\u{e9}",
        ".fifo",
        too_long.as_str(),
        fifo_too_long.as_str(),
    ] {
        s.err(
            json!({"Command": "CreateQueue", "QueueName": name}),
            "QueueNameInvalid",
        );
        s.err(
            json!({"Command": "CreateQueue", "QueueName": name, "Attributes": {"Fifo": true}}),
            "QueueNameInvalid",
        );
    }
}

#[test]
fn the_fifo_suffix_and_the_fifo_attribute_must_agree() {
    let mut s = Session::new();
    s.err(
        json!({"Command": "CreateQueue", "QueueName": "q.fifo"}),
        "QueueNameInvalid",
    );
    s.err(
        json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": {"Fifo": true}}),
        "QueueNameInvalid",
    );
    s.err(
        json!({"Command": "CreateQueue", "QueueName": "q.fifo", "Attributes": {"Fifo": false}}),
        "QueueNameInvalid",
    );
    create(&mut s, "q.fifo", json!({"Fifo": true}));
    assert_eq!(attributes(&mut s, "q.fifo")["Fifo"], true);
}

#[test]
fn queue_name_is_checked_before_other_parameters() {
    let mut s = Session::new();
    s.err(
        json!({"Command": "CreateQueue", "QueueName": "bad name", "Attributes": 5}),
        "QueueNameInvalid",
    );
    s.err(
        json!({"Command": "CreateQueue", "QueueName": 5, "Attributes": {"Bogus": 1}}),
        "InvalidParameterValue",
    );
    s.err(
        json!({"Command": "CreateQueue", "Attributes": {"Bogus": 1}}),
        "MissingParameter",
    );
}

#[test]
fn invalid_attributes_are_rejected() {
    let mut s = Session::new();
    let bad = [
        json!(5),
        json!([]),
        json!({"Bogus": 1}),
        json!({"Bogus": null}),
        json!({"VisibilityTimeout": -1}),
        json!({"VisibilityTimeout": 43201}),
        json!({"VisibilityTimeout": "30"}),
        json!({"DelaySeconds": 901}),
        json!({"RetentionSeconds": 59}),
        json!({"RetentionSeconds": 1209601}),
        json!({"Fifo": "true"}),
        json!({"ContentBasedDeduplication": 1}),
        json!({"DeadLetterQueue": 5, "MaxReceiveCount": 1}),
        json!({"MaxReceiveCount": 0, "DeadLetterQueue": "d"}),
        json!({"MaxReceiveCount": 1001, "DeadLetterQueue": "d"}),
        json!({"ContentBasedDeduplication": true}),
        json!({"DeadLetterQueue": "d"}),
        json!({"MaxReceiveCount": 3}),
    ];
    for attributes in bad {
        s.err(
            json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": attributes}),
            "InvalidParameterValue",
        );
    }
    s.ok(json!({"Command": "ListQueues"}));
    assert_eq!(names(&mut s, json!({"Command": "ListQueues"})), json!([]));
}

#[test]
fn null_attributes_are_absent() {
    let mut s = Session::new();
    create(
        &mut s,
        "q",
        json!({"VisibilityTimeout": null, "DeadLetterQueue": null, "Fifo": null}),
    );
    s.ok(json!({"Command": "CreateQueue", "QueueName": "q2", "Attributes": null}));
    assert_eq!(attributes(&mut s, "q")["VisibilityTimeout"], 30);
}

#[test]
fn creating_an_existing_queue_succeeds_only_with_identical_attributes() {
    let mut s = Session::new();
    create(&mut s, "q", json!({"VisibilityTimeout": 5}));
    create(
        &mut s,
        "q",
        json!({"VisibilityTimeout": 5, "DelaySeconds": 0}),
    );
    s.err(
        json!({"Command": "CreateQueue", "QueueName": "q"}),
        "QueueAlreadyExists",
    );
    s.err(
        json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": {"VisibilityTimeout": 6}}),
        "QueueAlreadyExists",
    );
    assert_eq!(attributes(&mut s, "q")["VisibilityTimeout"], 5);
    create(&mut s, "d", json!({}));
    create(
        &mut s,
        "r",
        json!({"DeadLetterQueue": "d", "MaxReceiveCount": 2}),
    );
    create(
        &mut s,
        "r",
        json!({"DeadLetterQueue": "d", "MaxReceiveCount": 2}),
    );
    s.err(
        json!({"Command": "CreateQueue", "QueueName": "r", "Attributes": {"DeadLetterQueue": "d", "MaxReceiveCount": 3}}),
        "QueueAlreadyExists",
    );
    s.err(
        json!({"Command": "CreateQueue", "QueueName": "r"}),
        "QueueAlreadyExists",
    );
}

#[test]
fn dead_letter_queue_must_exist_and_have_the_same_type() {
    let mut s = Session::new();
    s.err(
        json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": {"DeadLetterQueue": "nope", "MaxReceiveCount": 1}}),
        "QueueDoesNotExist",
    );
    create(&mut s, "d.fifo", json!({"Fifo": true}));
    s.err(
        json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": {"DeadLetterQueue": "d.fifo", "MaxReceiveCount": 1}}),
        "InvalidParameterValue",
    );
    create(
        &mut s,
        "q2.fifo",
        json!({"Fifo": true, "DeadLetterQueue": "d.fifo", "MaxReceiveCount": 1000}),
    );
    let a = attributes(&mut s, "q2.fifo");
    assert_eq!(a["DeadLetterQueue"], "d.fifo");
    assert_eq!(a["MaxReceiveCount"], 1000);
}

#[test]
fn at_most_1000_queues_can_exist() {
    let mut s = Session::new();
    for i in 0..1000 {
        s.ok(json!({"Command": "CreateQueue", "QueueName": format!("q{i}")}));
    }
    s.err(
        json!({"Command": "CreateQueue", "QueueName": "extra"}),
        "LimitExceeded",
    );
    // Re-creating an existing queue needs no room.
    s.ok(json!({"Command": "CreateQueue", "QueueName": "q7"}));
    s.ok(json!({"Command": "DeleteQueue", "QueueName": "q7"}));
    s.ok(json!({"Command": "CreateQueue", "QueueName": "extra"}));
}

#[test]
fn set_queue_attributes_changes_only_the_given_attributes() {
    let mut s = Session::new();
    create(&mut s, "q", json!({"DelaySeconds": 7}));
    s.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"VisibilityTimeout": 1, "RetentionSeconds": 60}}));
    let a = attributes(&mut s, "q");
    assert_eq!(a["VisibilityTimeout"], 1);
    assert_eq!(a["RetentionSeconds"], 60);
    assert_eq!(a["DelaySeconds"], 7);
    s.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {}}));
    assert_eq!(attributes(&mut s, "q"), a);
}

#[test]
fn set_queue_attributes_checks_in_the_documented_order() {
    let mut s = Session::new();
    create(&mut s, "q", json!({}));
    create(&mut s, "f.fifo", json!({"Fifo": true}));
    let set = |name: &str, attributes: Value| json!({"Command": "SetQueueAttributes", "QueueName": name, "Attributes": attributes});
    s.err(
        json!({"Command": "SetQueueAttributes", "QueueName": "q"}),
        "MissingParameter",
    );
    s.err(
        json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": null}),
        "MissingParameter",
    );
    s.err(set("q", json!([])), "InvalidParameterValue");
    s.err(set("missing", json!({"Fifo": true})), "QueueDoesNotExist");
    s.err(
        set("missing", json!({"Bogus": true})),
        "InvalidParameterValue",
    );
    s.err(set("q", json!({"Fifo": false})), "UnsupportedOperation");
    s.err(
        set(
            "q",
            json!({"Fifo": true, "ContentBasedDeduplication": true}),
        ),
        "UnsupportedOperation",
    );
    s.err(
        set("q", json!({"ContentBasedDeduplication": true})),
        "InvalidParameterValue",
    );
    s.ok(set("q", json!({"ContentBasedDeduplication": false})));
    s.ok(set("f.fifo", json!({"ContentBasedDeduplication": true})));
    s.err(
        set("q", json!({"DeadLetterQueue": "q"})),
        "InvalidParameterValue",
    );
    s.err(
        set("q", json!({"MaxReceiveCount": 2})),
        "InvalidParameterValue",
    );
    s.err(
        set(
            "q",
            json!({"DeadLetterQueue": "nope", "MaxReceiveCount": 2}),
        ),
        "QueueDoesNotExist",
    );
    s.err(
        set("q", json!({"DeadLetterQueue": "q", "MaxReceiveCount": 2})),
        "InvalidParameterValue",
    );
    s.err(
        set(
            "q",
            json!({"DeadLetterQueue": "f.fifo", "MaxReceiveCount": 2}),
        ),
        "InvalidParameterValue",
    );
    create(&mut s, "d", json!({}));
    s.ok(set(
        "q",
        json!({"DeadLetterQueue": "d", "MaxReceiveCount": 2}),
    ));
    s.ok(set("q", json!({"VisibilityTimeout": 9})));
    let a = attributes(&mut s, "q");
    assert_eq!(a["DeadLetterQueue"], "d");
    assert_eq!(a["MaxReceiveCount"], 2);
    assert_eq!(a["VisibilityTimeout"], 9);
    // Failed commands changed nothing.
    assert_eq!(attributes(&mut s, "f.fifo")["DeadLetterQueue"], Value::Null);
}

#[test]
fn list_queues_sorts_by_byte_value_and_filters_by_prefix() {
    let mut s = Session::new();
    for name in ["b", "a_", "Z", "a-", "a0", "_x", "a.fifo", "ab"] {
        let fifo = name.ends_with(".fifo");
        create(&mut s, name, json!({"Fifo": fifo}));
    }
    assert_eq!(
        names(&mut s, json!({"Command": "ListQueues"})),
        json!(["Z", "_x", "a-", "a.fifo", "a0", "a_", "ab", "b"])
    );
    assert_eq!(
        names(&mut s, json!({"Command": "ListQueues", "Prefix": "a"})),
        json!(["a-", "a.fifo", "a0", "a_", "ab"])
    );
    assert_eq!(
        names(&mut s, json!({"Command": "ListQueues", "Prefix": ""}))
            .as_array()
            .unwrap()
            .len(),
        8
    );
    assert_eq!(
        names(&mut s, json!({"Command": "ListQueues", "Prefix": null}))
            .as_array()
            .unwrap()
            .len(),
        8
    );
    assert_eq!(
        names(&mut s, json!({"Command": "ListQueues", "Prefix": "zz"})),
        json!([])
    );
    s.err(
        json!({"Command": "ListQueues", "Prefix": 1}),
        "InvalidParameterValue",
    );
}

#[test]
fn deleting_a_queue_unsets_dead_letter_configuration_pointing_at_it() {
    let mut s = Session::new();
    create(&mut s, "d", json!({}));
    create(
        &mut s,
        "q",
        json!({"DeadLetterQueue": "d", "MaxReceiveCount": 2}),
    );
    s.ok(json!({"Command": "DeleteQueue", "QueueName": "d"}));
    let a = attributes(&mut s, "q");
    assert_eq!(a.get("DeadLetterQueue"), None);
    assert_eq!(a.get("MaxReceiveCount"), None);
    s.err(
        json!({"Command": "DeleteQueue", "QueueName": "d"}),
        "QueueDoesNotExist",
    );
    create(&mut s, "d", json!({}));
    assert_eq!(attributes(&mut s, "q").get("DeadLetterQueue"), None);
}

#[test]
fn queue_lookup_is_exact_for_every_command() {
    let mut s = Session::new();
    create(&mut s, "q", json!({}));
    for name in ["Q", "q ", "", "q.fifo", "no such"] {
        for command in ["GetQueueAttributes", "DeleteQueue", "PurgeQueue"] {
            s.err(
                json!({"Command": command, "QueueName": name}),
                "QueueDoesNotExist",
            );
        }
    }
    for command in ["GetQueueAttributes", "DeleteQueue", "PurgeQueue"] {
        s.err(json!({"Command": command}), "MissingParameter");
        s.err(
            json!({"Command": command, "QueueName": 1}),
            "InvalidParameterValue",
        );
    }
}
