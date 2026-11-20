mod common;

use common::Miniq;
use serde_json::{Value, json};

fn create(name: &str, attributes: Value) -> Value {
    json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attributes})
}

fn attributes_of(miniq: &mut Miniq, name: &str) -> Value {
    miniq.ok(json!({"Command": "GetQueueAttributes", "QueueName": name}))["Attributes"].clone()
}

#[test]
fn create_with_defaults_and_get() {
    let mut miniq = Miniq::new();
    assert_eq!(
        miniq.ok(json!({"Command": "CreateQueue", "QueueName": "jobs"})),
        json!({"ok": true})
    );
    assert_eq!(
        miniq.ok(json!({"Command": "GetQueueAttributes", "QueueName": "jobs"})),
        json!({
            "ok": true,
            "Attributes": {
                "VisibilityTimeout": 30,
                "DelaySeconds": 0,
                "RetentionSeconds": 345600,
                "Fifo": false,
                "ContentBasedDeduplication": false,
            },
            "Counts": {"Visible": 0, "InFlight": 0, "Delayed": 0},
        })
    );
}

#[test]
fn create_with_every_attribute() {
    let mut miniq = Miniq::new();
    miniq.ok(create("dead", json!({})));
    miniq.ok(create(
        "jobs",
        json!({
            "VisibilityTimeout": 43200,
            "DelaySeconds": 900,
            "RetentionSeconds": 60,
            "DeadLetterQueue": "dead",
            "MaxReceiveCount": 1000,
        }),
    ));
    assert_eq!(
        attributes_of(&mut miniq, "jobs"),
        json!({
            "VisibilityTimeout": 43200,
            "DelaySeconds": 900,
            "RetentionSeconds": 60,
            "Fifo": false,
            "ContentBasedDeduplication": false,
            "DeadLetterQueue": "dead",
            "MaxReceiveCount": 1000,
        })
    );
}

#[test]
fn fifo_queue_with_content_based_deduplication() {
    let mut miniq = Miniq::new();
    miniq.ok(create(
        "a.fifo",
        json!({"Fifo": true, "ContentBasedDeduplication": true}),
    ));
    let attributes = attributes_of(&mut miniq, "a.fifo");
    assert_eq!(attributes["Fifo"], true);
    assert_eq!(attributes["ContentBasedDeduplication"], true);
}

#[test]
fn queue_name_syntax() {
    let max_plain = "a".repeat(80);
    let max_fifo = format!("{}.fifo", "b".repeat(75));
    let valid = ["a", "A-z_09", max_plain.as_str()];
    let mut miniq = Miniq::new();
    for name in valid {
        miniq.ok(json!({"Command": "CreateQueue", "QueueName": name}));
    }
    miniq.ok(create(&max_fifo, json!({"Fifo": true})));
    miniq.ok(create("x.fifo", json!({"Fifo": true})));

    let too_long_plain = "a".repeat(81);
    let too_long_fifo = format!("{}.fifo", "b".repeat(76));
    let invalid = [
        "",
        ".fifo",
        "a.b",
        "a b",
        "a/b",
        "é",
        "a.fifo.fifo",
        "a.FIFO",
        too_long_plain.as_str(),
        too_long_fifo.as_str(),
    ];
    for name in invalid {
        miniq.fails(
            json!({"Command": "CreateQueue", "QueueName": name}),
            "QueueNameInvalid",
        );
    }
}

#[test]
fn fifo_flag_and_name_suffix_must_agree() {
    let mut miniq = Miniq::new();
    miniq.fails(create("a.fifo", json!({})), "QueueNameInvalid");
    miniq.fails(create("a", json!({"Fifo": true})), "QueueNameInvalid");
    miniq.fails(create("a.fifo", json!({"Fifo": false})), "QueueNameInvalid");
    miniq.ok(create("a", json!({"Fifo": false})));
}

#[test]
fn create_parameter_errors_follow_the_documented_order() {
    let mut miniq = Miniq::new();
    miniq.fails(json!({"Command": "CreateQueue"}), "MissingParameter");
    miniq.fails(
        json!({"Command": "CreateQueue", "QueueName": null}),
        "MissingParameter",
    );
    miniq.fails(
        json!({"Command": "CreateQueue", "QueueName": 5}),
        "InvalidParameterValue",
    );
    // The name outranks bad attributes.
    miniq.fails(create("a b", json!({"Fifo": "yes"})), "QueueNameInvalid");
    miniq.fails(
        json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": []}),
        "InvalidParameterValue",
    );
    // A bad attribute outranks the Fifo/name rule.
    miniq.fails(
        create("a.fifo", json!({"DelaySeconds": 901})),
        "InvalidParameterValue",
    );
}

#[test]
fn attribute_values_are_checked_for_type_and_range() {
    let bad = [
        json!({"VisibilityTimeout": -1}),
        json!({"VisibilityTimeout": 43201}),
        json!({"VisibilityTimeout": 30.0}),
        json!({"VisibilityTimeout": "30"}),
        json!({"DelaySeconds": 901}),
        json!({"DelaySeconds": true}),
        json!({"RetentionSeconds": 59}),
        json!({"RetentionSeconds": 1209601}),
        json!({"Fifo": 1}),
        json!({"Fifo": "true"}),
        json!({"ContentBasedDeduplication": "true"}),
        json!({"DeadLetterQueue": 7, "MaxReceiveCount": 1}),
        json!({"DeadLetterQueue": "x", "MaxReceiveCount": 0}),
        json!({"DeadLetterQueue": "x", "MaxReceiveCount": 1001}),
        json!({"Colour": "red"}),
        json!({"visibilitytimeout": 5}),
    ];
    for attributes in bad {
        let mut miniq = Miniq::new();
        miniq.ok(json!({"Command": "CreateQueue", "QueueName": "x"}));
        miniq.fails(create("q", attributes.clone()), "InvalidParameterValue");
        miniq.fails(
            json!({"Command": "SetQueueAttributes", "QueueName": "x", "Attributes": attributes}),
            "InvalidParameterValue",
        );
    }
}

#[test]
fn integer_attributes_accept_only_plain_integer_syntax() {
    let mut miniq = Miniq::new();
    for text in ["3e1", "30.0", "3E1", "1.5"] {
        let line = format!(
            r#"{{"Command": "CreateQueue", "QueueName": "q", "Attributes": {{"VisibilityTimeout": {text}}}}}"#
        );
        let response = miniq.call(serde_json::from_str(&line).unwrap());
        common::assert_error(&response, "InvalidParameterValue");
    }
    let line =
        r#"{"Command": "CreateQueue", "QueueName": "q", "Attributes": {"VisibilityTimeout": -0}}"#;
    miniq.ok(serde_json::from_str(line).unwrap());
    assert_eq!(attributes_of(&mut miniq, "q")["VisibilityTimeout"], 0);
}

#[test]
fn null_attributes_are_absent() {
    let mut miniq = Miniq::new();
    miniq.ok(create(
        "q",
        json!({"VisibilityTimeout": null, "DeadLetterQueue": null, "MaxReceiveCount": null}),
    ));
    miniq.ok(json!({"Command": "CreateQueue", "QueueName": "r", "Attributes": null}));
    assert_eq!(attributes_of(&mut miniq, "q")["VisibilityTimeout"], 30);
}

#[test]
fn content_based_deduplication_needs_fifo() {
    let mut miniq = Miniq::new();
    miniq.fails(
        create("q", json!({"ContentBasedDeduplication": true})),
        "InvalidParameterValue",
    );
    miniq.ok(create("q", json!({"ContentBasedDeduplication": false})));
    miniq.fails(
        json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"ContentBasedDeduplication": true}}),
        "InvalidParameterValue",
    );
}

#[test]
fn dead_letter_attributes_come_in_pairs() {
    let mut miniq = Miniq::new();
    miniq.ok(create("dead", json!({})));
    miniq.fails(
        create("q", json!({"DeadLetterQueue": "dead"})),
        "InvalidParameterValue",
    );
    miniq.fails(
        create("q", json!({"MaxReceiveCount": 3})),
        "InvalidParameterValue",
    );
    miniq.fails(
        json!({"Command": "SetQueueAttributes", "QueueName": "dead", "Attributes": {"MaxReceiveCount": 3}}),
        "InvalidParameterValue",
    );
}

#[test]
fn creating_an_existing_queue_is_idempotent_only_for_equal_attributes() {
    let mut miniq = Miniq::new();
    miniq.ok(create("q", json!({"VisibilityTimeout": 5})));
    miniq.ok(create("q", json!({"VisibilityTimeout": 5})));
    miniq.ok(create(
        "q",
        json!({"VisibilityTimeout": 5, "DelaySeconds": 0, "Fifo": false}),
    ));
    miniq.fails(
        json!({"Command": "CreateQueue", "QueueName": "q"}),
        "QueueAlreadyExists",
    );
    miniq.fails(
        create("q", json!({"VisibilityTimeout": 6})),
        "QueueAlreadyExists",
    );

    miniq.ok(create("dead", json!({})));
    miniq.ok(create(
        "d",
        json!({"DeadLetterQueue": "dead", "MaxReceiveCount": 2}),
    ));
    miniq.ok(create(
        "d",
        json!({"DeadLetterQueue": "dead", "MaxReceiveCount": 2}),
    ));
    miniq.fails(
        create(
            "d",
            json!({"DeadLetterQueue": "dead", "MaxReceiveCount": 3}),
        ),
        "QueueAlreadyExists",
    );
    miniq.fails(
        json!({"Command": "CreateQueue", "QueueName": "d"}),
        "QueueAlreadyExists",
    );
}

#[test]
fn create_dead_letter_queue_rules() {
    let mut miniq = Miniq::new();
    miniq.fails(
        create(
            "q",
            json!({"DeadLetterQueue": "missing", "MaxReceiveCount": 1}),
        ),
        "QueueDoesNotExist",
    );
    miniq.ok(create("dead.fifo", json!({"Fifo": true})));
    miniq.fails(
        create(
            "q",
            json!({"DeadLetterQueue": "dead.fifo", "MaxReceiveCount": 1}),
        ),
        "InvalidParameterValue",
    );
    miniq.ok(create(
        "q.fifo",
        json!({"Fifo": true, "DeadLetterQueue": "dead.fifo", "MaxReceiveCount": 1}),
    ));
    miniq.fails(
        create(
            "q.fifo",
            json!({"Fifo": true, "DeadLetterQueue": "q.fifo", "MaxReceiveCount": 1}),
        ),
        "QueueAlreadyExists",
    );
}

#[test]
fn at_most_1000_queues() {
    let mut miniq = Miniq::new();
    for n in 0..1000 {
        miniq.ok(json!({"Command": "CreateQueue", "QueueName": format!("q{n}")}));
    }
    miniq.fails(
        json!({"Command": "CreateQueue", "QueueName": "extra"}),
        "LimitExceeded",
    );
    // Re-creating an existing queue needs no room.
    miniq.ok(json!({"Command": "CreateQueue", "QueueName": "q5"}));
    miniq.ok(json!({"Command": "DeleteQueue", "QueueName": "q5"}));
    miniq.ok(json!({"Command": "CreateQueue", "QueueName": "extra"}));
}

#[test]
fn set_attributes_changes_only_the_given_ones() {
    let mut miniq = Miniq::new();
    miniq.ok(create("dead", json!({})));
    miniq.ok(create("q", json!({"DelaySeconds": 7})));
    miniq.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {}}));
    assert_eq!(attributes_of(&mut miniq, "q")["DelaySeconds"], 7);
    miniq.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {
        "VisibilityTimeout": 9, "RetentionSeconds": 100, "DeadLetterQueue": "dead", "MaxReceiveCount": 4,
        "Fifo": null,
    }}));
    assert_eq!(
        attributes_of(&mut miniq, "q"),
        json!({
            "VisibilityTimeout": 9,
            "DelaySeconds": 7,
            "RetentionSeconds": 100,
            "Fifo": false,
            "ContentBasedDeduplication": false,
            "DeadLetterQueue": "dead",
            "MaxReceiveCount": 4,
        })
    );
    miniq.ok(create("other", json!({})));
    miniq.ok(
        json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {
            "DeadLetterQueue": "other", "MaxReceiveCount": 5,
        }}),
    );
    let attributes = attributes_of(&mut miniq, "q");
    assert_eq!(attributes["DeadLetterQueue"], "other");
    assert_eq!(attributes["MaxReceiveCount"], 5);
}

#[test]
fn set_attributes_rule_order() {
    let mut miniq = Miniq::new();
    miniq.ok(create("q", json!({})));
    miniq.ok(create("f.fifo", json!({"Fifo": true})));
    let set = |attributes: Value| json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": attributes});
    miniq.fails(
        json!({"Command": "SetQueueAttributes", "QueueName": "q"}),
        "MissingParameter",
    );
    miniq.fails(
        json!({"Command": "SetQueueAttributes", "QueueName": "nope", "Attributes": {"Fifo": 3}}),
        "InvalidParameterValue",
    );
    miniq.fails(
        json!({"Command": "SetQueueAttributes", "QueueName": "nope", "Attributes": {}}),
        "QueueDoesNotExist",
    );
    miniq.fails(set(json!({"Fifo": false})), "UnsupportedOperation");
    miniq.fails(
        set(json!({"Fifo": true, "ContentBasedDeduplication": true})),
        "UnsupportedOperation",
    );
    miniq.fails(
        set(json!({"DeadLetterQueue": "missing", "MaxReceiveCount": 1})),
        "QueueDoesNotExist",
    );
    miniq.fails(
        set(json!({"DeadLetterQueue": "q", "MaxReceiveCount": 1})),
        "InvalidParameterValue",
    );
    miniq.fails(
        set(json!({"DeadLetterQueue": "f.fifo", "MaxReceiveCount": 1})),
        "InvalidParameterValue",
    );
    miniq.fails(
        set(json!({"DeadLetterQueue": "f.fifo"})),
        "InvalidParameterValue",
    );
    // A failed command changes nothing.
    assert_eq!(attributes_of(&mut miniq, "q")["VisibilityTimeout"], 30);
}

#[test]
fn list_queues_is_sorted_by_byte_value_and_filtered_by_prefix() {
    let mut miniq = Miniq::new();
    for name in ["b", "a_1", "a-1", "A1", "a1", "a.fifo", "ab"] {
        let fifo = name.ends_with(".fifo");
        miniq.ok(create(name, json!({"Fifo": fifo})));
    }
    assert_eq!(
        miniq.ok(json!({"Command": "ListQueues"}))["QueueNames"],
        json!(["A1", "a-1", "a.fifo", "a1", "a_1", "ab", "b"])
    );
    assert_eq!(
        miniq.ok(json!({"Command": "ListQueues", "Prefix": "a"}))["QueueNames"],
        json!(["a-1", "a.fifo", "a1", "a_1", "ab"])
    );
    assert_eq!(
        miniq.ok(json!({"Command": "ListQueues", "Prefix": null}))["QueueNames"]
            .as_array()
            .unwrap()
            .len(),
        7
    );
    assert_eq!(
        miniq.ok(json!({"Command": "ListQueues", "Prefix": "zz"}))["QueueNames"],
        json!([])
    );
    miniq.fails(
        json!({"Command": "ListQueues", "Prefix": 1}),
        "InvalidParameterValue",
    );
}

#[test]
fn deleting_a_queue_clears_dead_letter_configuration_pointing_at_it() {
    let mut miniq = Miniq::new();
    miniq.ok(create("dead", json!({})));
    miniq.ok(create("keeps", json!({})));
    miniq.ok(create(
        "q",
        json!({"DeadLetterQueue": "dead", "MaxReceiveCount": 2}),
    ));
    miniq.ok(create(
        "r",
        json!({"DeadLetterQueue": "keeps", "MaxReceiveCount": 2}),
    ));
    miniq.ok(json!({"Command": "DeleteQueue", "QueueName": "dead"}));
    assert_eq!(attributes_of(&mut miniq, "q").get("DeadLetterQueue"), None);
    assert_eq!(attributes_of(&mut miniq, "q").get("MaxReceiveCount"), None);
    assert_eq!(attributes_of(&mut miniq, "r")["DeadLetterQueue"], "keeps");
    miniq.fails(
        json!({"Command": "DeleteQueue", "QueueName": "dead"}),
        "QueueDoesNotExist",
    );
    // The configuration may be set again, and the name reused.
    miniq.ok(create("dead", json!({})));
    miniq.ok(
        json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {
            "DeadLetterQueue": "dead", "MaxReceiveCount": 2,
        }}),
    );
    assert_eq!(attributes_of(&mut miniq, "q")["DeadLetterQueue"], "dead");
}

#[test]
fn unknown_queue_names_are_reported_as_missing_queues() {
    let mut miniq = Miniq::new();
    for command in ["GetQueueAttributes", "DeleteQueue", "PurgeQueue"] {
        for name in ["nope", "", "not a valid name!", "A.fifo"] {
            miniq.fails(
                json!({"Command": command, "QueueName": name}),
                "QueueDoesNotExist",
            );
        }
        miniq.fails(json!({"Command": command}), "MissingParameter");
        miniq.fails(
            json!({"Command": command, "QueueName": 3}),
            "InvalidParameterValue",
        );
    }
}

#[test]
fn tick_advances_the_clock() {
    let mut miniq = Miniq::new();
    assert_eq!(miniq.ok(json!({"Command": "Tick", "Seconds": 0}))["Now"], 0);
    assert_eq!(miniq.ok(json!({"Command": "Tick", "Seconds": 5}))["Now"], 5);
    assert_eq!(
        miniq.ok(json!({"Command": "Tick", "Seconds": 1209600}))["Now"],
        1209605
    );
    miniq.fails(json!({"Command": "Tick"}), "MissingParameter");
    for seconds in [
        json!(-1),
        json!(1209601),
        json!(1.0),
        json!("1"),
        json!(true),
    ] {
        miniq.fails(
            json!({"Command": "Tick", "Seconds": seconds}),
            "InvalidParameterValue",
        );
    }
    assert_eq!(
        miniq.ok(json!({"Command": "Tick", "Seconds": 0}))["Now"],
        1209605
    );
}

#[test]
fn unknown_members_are_ignored() {
    let mut miniq = Miniq::new();
    miniq.ok(json!({"Command": "Tick", "Seconds": 1, "Extra": [1, 2]}));
}
