mod common;

use common::{Session, code};
use serde_json::json;

#[test]
fn malformed_lines_are_invalid_requests() {
    let mut session = Session::new();
    let lines: [&[u8]; 9] = [
        b"",
        b"   ",
        b"not json",
        b"[]",
        b"\"Tick\"",
        b"{}",
        br#"{"Command": 1}"#,
        br#"{"Command": "NoSuchCommand"}"#,
        b"{\"Command\": \"Tick\", \"Seconds\": 1} trailing",
    ];
    for line in lines {
        let response = session.send_bytes(line);
        assert_eq!(code(&response), "InvalidRequest", "{line:?}");
    }
    assert_eq!(
        code(&session.send_bytes(b"{\"Command\": \"\xff\"}")),
        "InvalidRequest"
    );
}

#[test]
fn command_names_are_case_sensitive_and_extra_members_are_ignored() {
    let mut session = Session::new();
    assert_eq!(
        code(&session.send(r#"{"Command": "tick", "Seconds": 1}"#)),
        "InvalidRequest"
    );
    assert_eq!(
        session.send(r#"{"Command": "Tick", "Seconds": 5, "Unused": [1, 2]}"#),
        json!({"ok": true, "Now": 5})
    );
}

#[test]
fn null_parameters_are_absent() {
    let mut session = Session::new();
    session.create("a", json!(null));
    session.create("b", json!({"VisibilityTimeout": null}));
    assert_eq!(
        session.send(r#"{"Command": "ListQueues", "Prefix": null}"#),
        json!({"ok": true, "QueueNames": ["a", "b"]})
    );
    assert_eq!(
        code(&session.send(r#"{"Command": "Tick", "Seconds": null}"#)),
        "MissingParameter"
    );
}

#[test]
fn integers_must_be_written_without_fraction_or_exponent() {
    let mut session = Session::new();
    for seconds in ["30.0", "3e1", "\"30\"", "true", "-1", "1209601", "null"] {
        let line = format!(r#"{{"Command": "Tick", "Seconds": {seconds}}}"#);
        let expected = if seconds == "null" {
            "MissingParameter"
        } else {
            "InvalidParameterValue"
        };
        assert_eq!(code(&session.send(&line)), expected, "{seconds}");
    }
    assert_eq!(
        session.send(r#"{"Command": "Tick", "Seconds": 1209600}"#),
        json!({"ok": true, "Now": 1_209_600})
    );
}

#[test]
fn negative_zero_is_an_integer() {
    let mut session = Session::new();
    assert_eq!(
        session.send(r#"{"Command": "Tick", "Seconds": -0}"#),
        json!({"ok": true, "Now": 0})
    );
}

#[test]
fn clock_only_moves_on_tick() {
    let mut session = Session::new();
    assert_eq!(
        session.send(r#"{"Command": "Tick", "Seconds": 0}"#),
        json!({"ok": true, "Now": 0})
    );
    session.tick(7);
    assert_eq!(
        session.send(r#"{"Command": "Tick", "Seconds": 3}"#),
        json!({"ok": true, "Now": 10})
    );
}

#[test]
fn new_queue_reports_defaults() {
    let mut session = Session::new();
    session.create("jobs", json!({}));
    assert_eq!(
        session.send(r#"{"Command": "GetQueueAttributes", "QueueName": "jobs"}"#),
        json!({
            "ok": true,
            "Attributes": {
                "VisibilityTimeout": 30,
                "DelaySeconds": 0,
                "RetentionSeconds": 345_600,
                "Fifo": false,
                "ContentBasedDeduplication": false
            },
            "Counts": {"Visible": 0, "InFlight": 0, "Delayed": 0}
        })
    );
}

#[test]
fn create_is_idempotent_only_for_equal_attributes() {
    let mut session = Session::new();
    session.create("jobs", json!({"VisibilityTimeout": 10}));
    session.create("jobs", json!({"VisibilityTimeout": 10, "DelaySeconds": 0}));
    let different = session.command(
        json!({"Command": "CreateQueue", "QueueName": "jobs", "Attributes": {"VisibilityTimeout": 11}}),
    );
    assert_eq!(code(&different), "QueueAlreadyExists");
    let defaults = session.command(json!({"Command": "CreateQueue", "QueueName": "jobs"}));
    assert_eq!(code(&defaults), "QueueAlreadyExists");
}

#[test]
fn create_checks_the_name_before_anything_else() {
    let mut session = Session::new();
    let cases = [
        (json!({"QueueName": "bad name"}), "QueueNameInvalid"),
        (
            json!({"QueueName": "bad name", "Attributes": 5}),
            "QueueNameInvalid",
        ),
        (
            json!({"QueueName": "q", "Attributes": 5}),
            "InvalidParameterValue",
        ),
        (json!({"QueueName": 5}), "InvalidParameterValue"),
        (json!({}), "MissingParameter"),
        (json!({"QueueName": "q.fifo"}), "QueueNameInvalid"),
        (
            json!({"QueueName": "q", "Attributes": {"Fifo": true}}),
            "QueueNameInvalid",
        ),
        (
            json!({"QueueName": "q.fifo", "Attributes": {"Fifo": false}}),
            "QueueNameInvalid",
        ),
        (
            json!({"QueueName": "q", "Attributes": {"ContentBasedDeduplication": true}}),
            "InvalidParameterValue",
        ),
        (
            json!({"QueueName": "q", "Attributes": {"MaxReceiveCount": 3}}),
            "InvalidParameterValue",
        ),
        (
            json!({"QueueName": "q", "Attributes": {"Colour": 1}}),
            "InvalidParameterValue",
        ),
        (
            json!({"QueueName": "q", "Attributes": {"DelaySeconds": 901}}),
            "InvalidParameterValue",
        ),
        (
            json!({"QueueName": "q", "Attributes": {"RetentionSeconds": 59}}),
            "InvalidParameterValue",
        ),
        (
            json!({"QueueName": "q", "Attributes": {"Fifo": "yes"}}),
            "InvalidParameterValue",
        ),
        (
            json!({"QueueName": "q", "Attributes": {"DeadLetterQueue": "gone", "MaxReceiveCount": 3}}),
            "QueueDoesNotExist",
        ),
    ];
    for (mut command, expected) in cases {
        command["Command"] = json!("CreateQueue");
        assert_eq!(
            code(&session.command(command.clone())),
            expected,
            "{command}"
        );
    }
}

#[test]
fn create_dead_letter_queue_must_match_the_type() {
    let mut session = Session::new();
    session.create("dead", json!({}));
    session.create("dead.fifo", json!({"Fifo": true}));
    let wrong = session.command(json!({"Command": "CreateQueue", "QueueName": "q.fifo",
        "Attributes": {"Fifo": true, "DeadLetterQueue": "dead", "MaxReceiveCount": 2}}));
    assert_eq!(code(&wrong), "InvalidParameterValue");
    session.create(
        "q",
        json!({"DeadLetterQueue": "dead", "MaxReceiveCount": 2}),
    );
    assert_eq!(
        session.send(r#"{"Command": "GetQueueAttributes", "QueueName": "q"}"#)["Attributes"],
        json!({
            "VisibilityTimeout": 30, "DelaySeconds": 0, "RetentionSeconds": 345_600,
            "Fifo": false, "ContentBasedDeduplication": false,
            "DeadLetterQueue": "dead", "MaxReceiveCount": 2
        })
    );
    session.create(
        "q",
        json!({"DeadLetterQueue": "dead", "MaxReceiveCount": 2}),
    );
}

#[test]
fn at_most_1000_queues_exist() {
    let mut session = Session::new();
    for n in 0..1000 {
        session.create(&format!("q{n}"), json!({}));
    }
    session.create("q0", json!({}));
    let extra = session.command(json!({"Command": "CreateQueue", "QueueName": "extra"}));
    assert_eq!(code(&extra), "LimitExceeded");
    session.expect_ok(json!({"Command": "DeleteQueue", "QueueName": "q5"}));
    session.create("extra", json!({}));
}

#[test]
fn list_queues_sorts_by_byte_value_and_filters_by_prefix() {
    let mut session = Session::new();
    for name in ["b", "a_", "a.fifo", "A", "a-", "a1", "aa", "_"] {
        let attributes = if name.ends_with(".fifo") {
            json!({"Fifo": true})
        } else {
            json!({})
        };
        session.create(name, attributes);
    }
    assert_eq!(
        session.send(r#"{"Command": "ListQueues"}"#),
        json!({"ok": true, "QueueNames": ["A", "_", "a-", "a.fifo", "a1", "a_", "aa", "b"]})
    );
    assert_eq!(
        session.send(r#"{"Command": "ListQueues", "Prefix": "a"}"#),
        json!({"ok": true, "QueueNames": ["a-", "a.fifo", "a1", "a_", "aa"]})
    );
    assert_eq!(
        session.send(r#"{"Command": "ListQueues", "Prefix": "zz"}"#),
        json!({"ok": true, "QueueNames": []})
    );
    assert_eq!(
        code(&session.send(r#"{"Command": "ListQueues", "Prefix": 1}"#)),
        "InvalidParameterValue"
    );
}

#[test]
fn commands_on_unknown_queues_fail_after_parameter_checks() {
    let mut session = Session::new();
    let cases = [
        (
            json!({"Command": "GetQueueAttributes", "QueueName": "nope"}),
            "QueueDoesNotExist",
        ),
        (
            json!({"Command": "GetQueueAttributes", "QueueName": ""}),
            "QueueDoesNotExist",
        ),
        (
            json!({"Command": "GetQueueAttributes", "QueueName": "!"}),
            "QueueDoesNotExist",
        ),
        (json!({"Command": "GetQueueAttributes"}), "MissingParameter"),
        (
            json!({"Command": "GetQueueAttributes", "QueueName": 3}),
            "InvalidParameterValue",
        ),
        (
            json!({"Command": "DeleteQueue", "QueueName": "nope"}),
            "QueueDoesNotExist",
        ),
        (
            json!({"Command": "PurgeQueue", "QueueName": "nope"}),
            "QueueDoesNotExist",
        ),
        (
            json!({"Command": "SetQueueAttributes", "QueueName": "nope"}),
            "MissingParameter",
        ),
        (
            json!({"Command": "SetQueueAttributes", "QueueName": "nope", "Attributes": {"Bad": 1}}),
            "InvalidParameterValue",
        ),
        (
            json!({"Command": "SetQueueAttributes", "QueueName": "nope", "Attributes": {"Fifo": true}}),
            "QueueDoesNotExist",
        ),
    ];
    for (command, expected) in cases {
        assert_eq!(
            code(&session.command(command.clone())),
            expected,
            "{command}"
        );
    }
}

#[test]
fn set_queue_attributes_updates_only_what_is_given() {
    let mut session = Session::new();
    session.create("q", json!({"VisibilityTimeout": 5}));
    session.expect_ok(json!({"Command": "SetQueueAttributes", "QueueName": "q",
        "Attributes": {"DelaySeconds": 9, "RetentionSeconds": 100}}));
    session.expect_ok(json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {}}));
    assert_eq!(
        session.send(r#"{"Command": "GetQueueAttributes", "QueueName": "q"}"#)["Attributes"],
        json!({"VisibilityTimeout": 5, "DelaySeconds": 9, "RetentionSeconds": 100,
            "Fifo": false, "ContentBasedDeduplication": false})
    );
}

#[test]
fn set_queue_attributes_applies_its_rules_in_order() {
    let mut session = Session::new();
    session.create("std", json!({}));
    session.create("other", json!({}));
    session.create("f.fifo", json!({"Fifo": true}));
    let cases = [
        (json!({"Fifo": false}), "UnsupportedOperation"),
        (
            json!({"Fifo": true, "ContentBasedDeduplication": true}),
            "UnsupportedOperation",
        ),
        (
            json!({"ContentBasedDeduplication": true, "MaxReceiveCount": 2}),
            "InvalidParameterValue",
        ),
        (json!({"MaxReceiveCount": 2}), "InvalidParameterValue"),
        (
            json!({"DeadLetterQueue": "gone", "MaxReceiveCount": 2}),
            "QueueDoesNotExist",
        ),
        (
            json!({"DeadLetterQueue": "std", "MaxReceiveCount": 2}),
            "InvalidParameterValue",
        ),
        (
            json!({"DeadLetterQueue": "f.fifo", "MaxReceiveCount": 2}),
            "InvalidParameterValue",
        ),
        (
            json!({"DeadLetterQueue": "other", "MaxReceiveCount": 2}),
            "<ok>",
        ),
        (
            json!({"DeadLetterQueue": "other", "MaxReceiveCount": 3, "ContentBasedDeduplication": false}),
            "<ok>",
        ),
    ];
    for (attributes, expected) in cases {
        let command =
            json!({"Command": "SetQueueAttributes", "QueueName": "std", "Attributes": attributes});
        assert_eq!(
            code(&session.command(command.clone())),
            expected,
            "{command}"
        );
    }
    let failed = session.command(
        json!({"Command": "SetQueueAttributes", "QueueName": "f.fifo",
        "Attributes": {"VisibilityTimeout": 1, "DeadLetterQueue": "gone", "MaxReceiveCount": 2}}),
    );
    assert_eq!(code(&failed), "QueueDoesNotExist");
    assert_eq!(
        session.send(r#"{"Command": "GetQueueAttributes", "QueueName": "f.fifo"}"#)["Attributes"]["VisibilityTimeout"],
        json!(30)
    );
    assert_eq!(
        session.send(r#"{"Command": "GetQueueAttributes", "QueueName": "std"}"#)["Attributes"]["MaxReceiveCount"],
        json!(3)
    );
}

#[test]
fn fifo_queue_can_toggle_content_based_deduplication() {
    let mut session = Session::new();
    session.create("f.fifo", json!({"Fifo": true}));
    session.expect_ok(
        json!({"Command": "SetQueueAttributes", "QueueName": "f.fifo",
        "Attributes": {"ContentBasedDeduplication": true}}),
    );
    assert_eq!(
        session.send(r#"{"Command": "GetQueueAttributes", "QueueName": "f.fifo"}"#)["Attributes"]["ContentBasedDeduplication"],
        json!(true)
    );
}

#[test]
fn deleting_a_queue_clears_dead_letter_configurations_that_name_it() {
    let mut session = Session::new();
    session.create("dead", json!({}));
    session.create(
        "q",
        json!({"DeadLetterQueue": "dead", "MaxReceiveCount": 4}),
    );
    session.expect_ok(json!({"Command": "DeleteQueue", "QueueName": "dead"}));
    assert_eq!(
        session.send(r#"{"Command": "GetQueueAttributes", "QueueName": "q"}"#)["Attributes"],
        json!({"VisibilityTimeout": 30, "DelaySeconds": 0, "RetentionSeconds": 345_600,
            "Fifo": false, "ContentBasedDeduplication": false})
    );
    let recreated = session.command(json!({"Command": "CreateQueue", "QueueName": "q",
        "Attributes": {"DeadLetterQueue": "dead", "MaxReceiveCount": 4}}));
    assert_eq!(code(&recreated), "QueueAlreadyExists");
    assert_eq!(
        code(&session.send(r#"{"Command": "GetQueueAttributes", "QueueName": "dead"}"#)),
        "QueueDoesNotExist"
    );
}
