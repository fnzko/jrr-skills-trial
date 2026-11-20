mod common;

use common::{error_code, ok, outcomes, session};
use serde_json::json;

fn create(name: &str) -> serde_json::Value {
    json!({"Command": "CreateQueue", "QueueName": name})
}

fn get(name: &str) -> serde_json::Value {
    json!({"Command": "GetQueueAttributes", "QueueName": name})
}

#[test]
fn a_new_queue_has_default_attributes_and_no_messages() {
    let responses = session(&[create("jobs"), get("jobs")]);
    assert_eq!(responses[0], ok());
    assert_eq!(
        responses[1],
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
fn given_attributes_are_kept_and_null_attributes_are_absent() {
    let responses = session(&[
        create("dead"),
        json!({"Command": "CreateQueue", "QueueName": "jobs", "Attributes": {
            "VisibilityTimeout": 0, "DelaySeconds": 900, "RetentionSeconds": 60,
            "DeadLetterQueue": "dead", "MaxReceiveCount": 1000, "Fifo": null,
        }}),
        get("jobs"),
    ]);
    assert_eq!(responses[1], ok());
    assert_eq!(
        responses[2]["Attributes"],
        json!({
            "VisibilityTimeout": 0,
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
fn creating_a_queue_again_succeeds_only_with_the_same_attributes() {
    let responses = session(&[
        create("jobs"),
        create("jobs"),
        json!({"Command": "CreateQueue", "QueueName": "jobs",
               "Attributes": {"VisibilityTimeout": 30, "Fifo": false}}),
        json!({"Command": "CreateQueue", "QueueName": "jobs",
               "Attributes": {"VisibilityTimeout": 31}}),
    ]);
    assert_eq!(responses[1], ok());
    assert_eq!(responses[2], ok());
    assert_eq!(error_code(&responses[3]), "QueueAlreadyExists");
}

#[test]
fn queue_names_follow_the_naming_rules() {
    let long = "a".repeat(80);
    let stem75 = format!("{}.fifo", "a".repeat(75));
    let accepted = [
        ("a", false),
        ("A_z-9", false),
        (long.as_str(), false),
        ("jobs.fifo", true),
        (stem75.as_str(), true),
    ];
    for (name, fifo) in accepted {
        let responses = session(&[json!({"Command": "CreateQueue", "QueueName": name,
                                         "Attributes": {"Fifo": fifo}})]);
        assert_eq!(responses[0], ok(), "name {name:?}");
    }

    let too_long = "a".repeat(81);
    let stem76 = format!("{}.fifo", "a".repeat(76));
    let rejected = [
        ("", false),
        ("a b", false),
        ("a.b", false),
        ("é", false),
        (too_long.as_str(), false),
        (".fifo", true),
        ("a.fifo.fifo", true),
        (stem76.as_str(), true),
        ("jobs.fifo", false),
        ("jobs", true),
    ];
    for (name, fifo) in rejected {
        let responses = session(&[json!({"Command": "CreateQueue", "QueueName": name,
                                         "Attributes": {"Fifo": fifo}})]);
        assert_eq!(
            error_code(&responses[0]),
            "QueueNameInvalid",
            "name {name:?}"
        );
    }
}

#[test]
fn create_queue_checks_run_in_the_documented_order() {
    let responses = session(&[
        create("dead"),
        // The name is checked before any other parameter.
        json!({"Command": "CreateQueue", "QueueName": "a b", "Attributes": 3}),
        // A missing parameter beats a wrong value.
        json!({"Command": "CreateQueue", "Attributes": 3}),
        json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": 3}),
        json!({"Command": "CreateQueue", "QueueName": 7}),
        // Name and Fifo disagree before ContentBasedDeduplication is looked at.
        json!({"Command": "CreateQueue", "QueueName": "q.fifo",
               "Attributes": {"ContentBasedDeduplication": true, "DeadLetterQueue": "x"}}),
        json!({"Command": "CreateQueue", "QueueName": "q",
               "Attributes": {"ContentBasedDeduplication": true, "DeadLetterQueue": "x"}}),
        json!({"Command": "CreateQueue", "QueueName": "q",
               "Attributes": {"DeadLetterQueue": "x"}}),
        json!({"Command": "CreateQueue", "QueueName": "q",
               "Attributes": {"DeadLetterQueue": "x", "MaxReceiveCount": 1}}),
        json!({"Command": "CreateQueue", "QueueName": "q.fifo",
               "Attributes": {"Fifo": true, "DeadLetterQueue": "dead", "MaxReceiveCount": 1}}),
    ]);
    assert_eq!(
        outcomes(&responses[1..]),
        [
            "QueueNameInvalid",
            "MissingParameter",
            "InvalidParameterValue",
            "InvalidParameterValue",
            "QueueNameInvalid",
            "InvalidParameterValue",
            "InvalidParameterValue",
            "QueueDoesNotExist",
            "InvalidParameterValue",
        ]
    );
}

#[test]
fn bad_attributes_are_rejected() {
    let bad = [
        json!({"VisibilityTimeout": 43201}),
        json!({"VisibilityTimeout": -1}),
        json!({"VisibilityTimeout": "30"}),
        json!({"DelaySeconds": 901}),
        json!({"RetentionSeconds": 59}),
        json!({"RetentionSeconds": 1209601}),
        json!({"MaxReceiveCount": 0, "DeadLetterQueue": "q"}),
        json!({"MaxReceiveCount": 1001, "DeadLetterQueue": "q"}),
        json!({"Fifo": "true"}),
        json!({"ContentBasedDeduplication": 1}),
        json!({"DeadLetterQueue": 5, "MaxReceiveCount": 1}),
        json!({"Colour": "red"}),
    ];
    for attributes in bad {
        let responses = session(&[
            create("q"),
            json!({"Command": "CreateQueue", "QueueName": "other", "Attributes": attributes}),
        ]);
        assert_eq!(
            error_code(&responses[1]),
            "InvalidParameterValue",
            "{attributes}"
        );
    }
}

#[test]
fn integers_with_a_fraction_or_exponent_are_rejected() {
    let responses = common::run(&[
        r#"{"Command":"CreateQueue","QueueName":"a","Attributes":{"VisibilityTimeout":30.0}}"#,
        r#"{"Command":"CreateQueue","QueueName":"b","Attributes":{"VisibilityTimeout":3e1}}"#,
    ]);
    assert_eq!(error_code(&responses[0]), "InvalidParameterValue");
    assert_eq!(error_code(&responses[1]), "InvalidParameterValue");
}

#[test]
fn a_dead_letter_queue_must_exist_and_have_the_same_type() {
    let responses = session(&[
        create("plain"),
        json!({"Command": "CreateQueue", "QueueName": "missing-dlq",
               "Attributes": {"DeadLetterQueue": "nope", "MaxReceiveCount": 2}}),
        json!({"Command": "CreateQueue", "QueueName": "f.fifo",
               "Attributes": {"Fifo": true, "DeadLetterQueue": "plain", "MaxReceiveCount": 2}}),
        json!({"Command": "CreateQueue", "QueueName": "dead.fifo", "Attributes": {"Fifo": true}}),
        json!({"Command": "CreateQueue", "QueueName": "std",
               "Attributes": {"DeadLetterQueue": "dead.fifo", "MaxReceiveCount": 2}}),
    ]);
    assert_eq!(error_code(&responses[1]), "QueueDoesNotExist");
    assert_eq!(error_code(&responses[2]), "InvalidParameterValue");
    assert_eq!(error_code(&responses[4]), "InvalidParameterValue");
}

#[test]
fn at_most_1000_queues_can_exist() {
    let mut commands: Vec<_> = (0..1000).map(|n| create(&format!("q{n}"))).collect();
    commands.push(create("q0"));
    commands.push(create("one-too-many"));
    let responses = session(&commands);
    assert!(responses[..1001].iter().all(|response| *response == ok()));
    assert_eq!(error_code(&responses[1001]), "LimitExceeded");
}

#[test]
fn list_queues_sorts_by_byte_value_and_filters_by_prefix() {
    let names = ["b", "a_", "a-", "A", "a1", "a.fifo", "ab"];
    let mut commands: Vec<_> = names
        .iter()
        .map(|name| {
            json!({"Command": "CreateQueue", "QueueName": name,
                   "Attributes": {"Fifo": name.ends_with(".fifo")}})
        })
        .collect();
    commands.push(json!({"Command": "ListQueues"}));
    commands.push(json!({"Command": "ListQueues", "Prefix": "a"}));
    commands.push(json!({"Command": "ListQueues", "Prefix": "a."}));
    commands.push(json!({"Command": "ListQueues", "Prefix": "zzz"}));
    commands.push(json!({"Command": "ListQueues", "Prefix": 1}));
    let responses = session(&commands);
    assert_eq!(
        responses[7]["QueueNames"],
        json!(["A", "a-", "a.fifo", "a1", "a_", "ab", "b"])
    );
    assert_eq!(
        responses[8]["QueueNames"],
        json!(["a-", "a.fifo", "a1", "a_", "ab"])
    );
    assert_eq!(responses[9]["QueueNames"], json!(["a.fifo"]));
    assert_eq!(responses[10], json!({"ok": true, "QueueNames": []}));
    assert_eq!(error_code(&responses[11]), "InvalidParameterValue");
}

#[test]
fn commands_on_a_missing_queue_fail_whatever_the_name_looks_like() {
    for name in ["nope", "", "not a valid name!", "jobs.fifo"] {
        let responses = session(&[
            create("jobs"),
            get(name),
            json!({"Command": "DeleteQueue", "QueueName": name}),
            json!({"Command": "PurgeQueue", "QueueName": name}),
            json!({"Command": "SetQueueAttributes", "QueueName": name, "Attributes": {}}),
            json!({"Command": "SendMessage", "QueueName": name, "Body": "x"}),
            json!({"Command": "ReceiveMessage", "QueueName": name}),
            json!({"Command": "DeleteMessage", "QueueName": name, "ReceiptHandle": "bogus"}),
        ]);
        for response in &responses[1..] {
            assert_eq!(error_code(response), "QueueDoesNotExist", "name {name:?}");
        }
    }
}

#[test]
fn deleting_a_queue_clears_dead_letter_configs_that_name_it() {
    let responses = session(&[
        create("dead"),
        json!({"Command": "CreateQueue", "QueueName": "jobs",
               "Attributes": {"DeadLetterQueue": "dead", "MaxReceiveCount": 3}}),
        json!({"Command": "SendMessage", "QueueName": "dead", "Body": "old"}),
        json!({"Command": "DeleteQueue", "QueueName": "dead"}),
        get("jobs"),
        create("dead"),
        get("dead"),
        json!({"Command": "DeleteQueue", "QueueName": "dead"}),
    ]);
    assert_eq!(responses[3], ok());
    assert_eq!(
        responses[4]["Attributes"],
        json!({
            "VisibilityTimeout": 30,
            "DelaySeconds": 0,
            "RetentionSeconds": 345600,
            "Fifo": false,
            "ContentBasedDeduplication": false,
        })
    );
    assert_eq!(
        responses[6]["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn set_queue_attributes_changes_only_the_named_attributes() {
    let responses = session(&[
        json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": {"DelaySeconds": 5}}),
        json!({"Command": "SetQueueAttributes", "QueueName": "q",
               "Attributes": {"VisibilityTimeout": 7, "RetentionSeconds": null}}),
        get("q"),
        json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {}}),
        get("q"),
    ]);
    assert_eq!(responses[1], ok());
    let expected = json!({
        "VisibilityTimeout": 7,
        "DelaySeconds": 5,
        "RetentionSeconds": 345600,
        "Fifo": false,
        "ContentBasedDeduplication": false,
    });
    assert_eq!(responses[2]["Attributes"], expected);
    assert_eq!(responses[3], ok());
    assert_eq!(responses[4]["Attributes"], expected);
}

#[test]
fn set_queue_attributes_follows_its_rule_order() {
    let responses = session(&[
        create("q"),
        create("other"),
        json!({"Command": "CreateQueue", "QueueName": "f.fifo", "Attributes": {"Fifo": true}}),
        json!({"Command": "SetQueueAttributes", "QueueName": "q"}),
        json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": []}),
        json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"Nope": 1}}),
        json!({"Command": "SetQueueAttributes", "QueueName": "q",
               "Attributes": {"Fifo": false, "ContentBasedDeduplication": true}}),
        json!({"Command": "SetQueueAttributes", "QueueName": "q",
               "Attributes": {"ContentBasedDeduplication": true, "DeadLetterQueue": "nope"}}),
        json!({"Command": "SetQueueAttributes", "QueueName": "q",
               "Attributes": {"MaxReceiveCount": 2}}),
        json!({"Command": "SetQueueAttributes", "QueueName": "q",
               "Attributes": {"DeadLetterQueue": "nope", "MaxReceiveCount": 2}}),
        json!({"Command": "SetQueueAttributes", "QueueName": "q",
               "Attributes": {"DeadLetterQueue": "q", "MaxReceiveCount": 2}}),
        json!({"Command": "SetQueueAttributes", "QueueName": "q",
               "Attributes": {"DeadLetterQueue": "f.fifo", "MaxReceiveCount": 2}}),
        json!({"Command": "SetQueueAttributes", "QueueName": "q",
               "Attributes": {"DeadLetterQueue": "other", "MaxReceiveCount": 2}}),
        json!({"Command": "SetQueueAttributes", "QueueName": "q",
               "Attributes": {"DeadLetterQueue": "other", "MaxReceiveCount": 9}}),
        get("q"),
    ]);
    assert_eq!(
        outcomes(&responses[3..13]),
        [
            "MissingParameter",
            "InvalidParameterValue",
            "InvalidParameterValue",
            "UnsupportedOperation",
            "InvalidParameterValue",
            "InvalidParameterValue",
            "QueueDoesNotExist",
            "InvalidParameterValue",
            "InvalidParameterValue",
            "ok",
        ]
    );
    assert_eq!(responses[13], ok());
    assert_eq!(responses[14]["Attributes"]["MaxReceiveCount"], 9);
}

#[test]
fn a_dead_letter_configuration_cannot_be_removed() {
    let responses = session(&[
        create("dead"),
        json!({"Command": "CreateQueue", "QueueName": "q",
               "Attributes": {"DeadLetterQueue": "dead", "MaxReceiveCount": 2}}),
        json!({"Command": "SetQueueAttributes", "QueueName": "q",
               "Attributes": {"DeadLetterQueue": null, "MaxReceiveCount": null}}),
        get("q"),
    ]);
    assert_eq!(responses[2], ok());
    assert_eq!(responses[3]["Attributes"]["DeadLetterQueue"], "dead");
}

#[test]
fn tick_advances_the_clock_by_the_given_seconds() {
    let responses = session(&[
        json!({"Command": "Tick", "Seconds": 0}),
        json!({"Command": "Tick", "Seconds": 1209600}),
        json!({"Command": "Tick", "Seconds": 1209601}),
        json!({"Command": "Tick", "Seconds": 1}),
    ]);
    assert_eq!(responses[0]["Now"], 0);
    assert_eq!(responses[1]["Now"], 1209600);
    assert_eq!(error_code(&responses[2]), "InvalidParameterValue");
    assert_eq!(responses[3]["Now"], 1209601);
}
