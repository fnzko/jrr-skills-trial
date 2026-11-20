mod common;

use common::{assert_error, assert_ok, last, run, session};
use serde_json::json;

#[test]
fn create_queue_with_defaults_succeeds_with_no_result_members() {
    let response = last(&[r#"{"Command": "CreateQueue", "QueueName": "jobs"}"#]);

    assert_eq!(response, json!({"ok": true}));
}

#[test]
fn creating_an_existing_queue_with_the_same_attributes_changes_nothing() {
    let responses = session(&[
        r#"{"Command": "CreateQueue", "QueueName": "jobs"}"#,
        r#"{"Command": "CreateQueue", "QueueName": "jobs", "Attributes": {"VisibilityTimeout": 30}}"#,
    ]);

    assert_ok(&responses[1]);
}

#[test]
fn creating_an_existing_queue_with_other_attributes_fails() {
    let responses = session(&[
        r#"{"Command": "CreateQueue", "QueueName": "jobs"}"#,
        r#"{"Command": "CreateQueue", "QueueName": "jobs", "Attributes": {"DelaySeconds": 5}}"#,
    ]);

    assert_error(&responses[1], "QueueAlreadyExists");
}

#[test]
fn queue_name_rules_decide_validity() {
    let long = "a".repeat(80);
    let too_long = "a".repeat(81);
    let fifo_max = format!("{}.fifo", "a".repeat(75));
    let fifo_too_long = format!("{}.fifo", "a".repeat(76));
    let cases: Vec<(String, bool, Option<&str>)> = vec![
        ("a".to_owned(), false, None),
        ("A-z_09".to_owned(), false, None),
        (long, false, None),
        (too_long, false, Some("QueueNameInvalid")),
        ("".to_owned(), false, Some("QueueNameInvalid")),
        ("a.b".to_owned(), false, Some("QueueNameInvalid")),
        ("a b".to_owned(), false, Some("QueueNameInvalid")),
        ("é".to_owned(), false, Some("QueueNameInvalid")),
        ("jobs.fifo".to_owned(), false, Some("QueueNameInvalid")),
        ("jobs".to_owned(), true, Some("QueueNameInvalid")),
        ("jobs.fifo".to_owned(), true, None),
        (fifo_max, true, None),
        (fifo_too_long, true, Some("QueueNameInvalid")),
    ];

    for (name, fifo, expected) in cases {
        let line = format!(
            r#"{{"Command": "CreateQueue", "QueueName": {}, "Attributes": {{"Fifo": {fifo}}}}}"#,
            serde_json::to_string(&name).unwrap()
        );
        let response = last(&[&line]);
        match expected {
            None => assert_ok(&response),
            Some(code) => assert_error(&response, code),
        }
    }
}

#[test]
fn queue_name_is_checked_before_attributes() {
    let response = last(&[
        r#"{"Command": "CreateQueue", "QueueName": "bad name", "Attributes": {"VisibilityTimeout": 99999}}"#,
    ]);

    assert_error(&response, "QueueNameInvalid");
}

#[test]
fn missing_queue_name_is_reported_before_bad_attributes() {
    let response =
        last(&[r#"{"Command": "CreateQueue", "Attributes": {"VisibilityTimeout": "x"}}"#]);

    assert_error(&response, "MissingParameter");
}

#[test]
fn non_string_queue_name_is_an_invalid_parameter_value() {
    let response = last(&[r#"{"Command": "CreateQueue", "QueueName": 5}"#]);

    assert_error(&response, "InvalidParameterValue");
}

#[test]
fn attribute_values_are_checked_by_type_and_range() {
    let cases = [
        (r#"{"VisibilityTimeout": 43200}"#, true),
        (r#"{"VisibilityTimeout": 43201}"#, false),
        (r#"{"VisibilityTimeout": "30"}"#, false),
        (r#"{"VisibilityTimeout": 30.0}"#, false),
        (r#"{"VisibilityTimeout": 3e1}"#, false),
        (r#"{"VisibilityTimeout": -1}"#, false),
        (r#"{"DelaySeconds": 900}"#, true),
        (r#"{"DelaySeconds": 901}"#, false),
        (r#"{"RetentionSeconds": 59}"#, false),
        (r#"{"RetentionSeconds": 60}"#, true),
        (r#"{"RetentionSeconds": 1209600}"#, true),
        (r#"{"RetentionSeconds": 1209601}"#, false),
        (r#"{"Fifo": "true"}"#, false),
        (r#"{"ContentBasedDeduplication": 1}"#, false),
        (r#"{"Color": 1}"#, false),
        (r#"{"VisibilityTimeout": null}"#, true),
        (r#"{"DelaySeconds": null, "VisibilityTimeout": 1}"#, true),
    ];

    for (attributes, valid) in cases {
        let line = format!(
            r#"{{"Command": "CreateQueue", "QueueName": "q-{}", "Attributes": {attributes}}}"#,
            attributes.len()
        );
        let response = last(&[&line]);
        if valid {
            assert_ok(&response);
        } else {
            assert_error(&response, "InvalidParameterValue");
        }
    }
}

#[test]
fn attributes_must_be_an_object() {
    let responses = session(&[
        r#"{"Command": "CreateQueue", "QueueName": "a", "Attributes": []}"#,
        r#"{"Command": "CreateQueue", "QueueName": "b", "Attributes": "x"}"#,
        r#"{"Command": "CreateQueue", "QueueName": "c", "Attributes": null}"#,
    ]);

    assert_error(&responses[0], "InvalidParameterValue");
    assert_error(&responses[1], "InvalidParameterValue");
    assert_ok(&responses[2]);
}

#[test]
fn content_based_deduplication_requires_a_fifo_queue() {
    let responses = session(&[
        r#"{"Command": "CreateQueue", "QueueName": "std", "Attributes": {"ContentBasedDeduplication": true}}"#,
        r#"{"Command": "CreateQueue", "QueueName": "std2", "Attributes": {"ContentBasedDeduplication": false}}"#,
        r#"{"Command": "CreateQueue", "QueueName": "f.fifo", "Attributes": {"Fifo": true, "ContentBasedDeduplication": true}}"#,
    ]);

    assert_error(&responses[0], "InvalidParameterValue");
    assert_ok(&responses[1]);
    assert_ok(&responses[2]);
}

#[test]
fn dead_letter_members_must_be_given_together() {
    let responses = session(&[
        r#"{"Command": "CreateQueue", "QueueName": "dlq"}"#,
        r#"{"Command": "CreateQueue", "QueueName": "a", "Attributes": {"DeadLetterQueue": "dlq"}}"#,
        r#"{"Command": "CreateQueue", "QueueName": "b", "Attributes": {"MaxReceiveCount": 3}}"#,
    ]);

    assert_error(&responses[1], "InvalidParameterValue");
    assert_error(&responses[2], "InvalidParameterValue");
}

#[test]
fn max_receive_count_is_checked_by_range() {
    let responses = session(&[
        r#"{"Command": "CreateQueue", "QueueName": "dlq"}"#,
        r#"{"Command": "CreateQueue", "QueueName": "a", "Attributes": {"DeadLetterQueue": "dlq", "MaxReceiveCount": 0}}"#,
        r#"{"Command": "CreateQueue", "QueueName": "b", "Attributes": {"DeadLetterQueue": "dlq", "MaxReceiveCount": 1001}}"#,
        r#"{"Command": "CreateQueue", "QueueName": "c", "Attributes": {"DeadLetterQueue": "dlq", "MaxReceiveCount": 1000}}"#,
    ]);

    assert_error(&responses[1], "InvalidParameterValue");
    assert_error(&responses[2], "InvalidParameterValue");
    assert_ok(&responses[3]);
}

#[test]
fn dead_letter_queue_must_exist() {
    let response = last(&[
        r#"{"Command": "CreateQueue", "QueueName": "a", "Attributes": {"DeadLetterQueue": "nope", "MaxReceiveCount": 3}}"#,
    ]);

    assert_error(&response, "QueueDoesNotExist");
}

#[test]
fn dead_letter_queue_must_have_the_same_fifo_type() {
    let response = last(&[
        r#"{"Command": "CreateQueue", "QueueName": "dlq"}"#,
        r#"{"Command": "CreateQueue", "QueueName": "f.fifo", "Attributes": {"Fifo": true, "DeadLetterQueue": "dlq", "MaxReceiveCount": 3}}"#,
    ]);

    assert_error(&response, "InvalidParameterValue");
}

#[test]
fn a_queue_cannot_be_its_own_dead_letter_queue_on_create() {
    let response = last(&[
        r#"{"Command": "CreateQueue", "QueueName": "a", "Attributes": {"DeadLetterQueue": "a", "MaxReceiveCount": 3}}"#,
    ]);

    assert_error(&response, "QueueDoesNotExist");
}

#[test]
fn queue_count_is_limited_to_1000() {
    let mut lines: Vec<String> = (0..1000)
        .map(|n| format!(r#"{{"Command": "CreateQueue", "QueueName": "q{n}"}}"#))
        .collect();
    lines.push(r#"{"Command": "CreateQueue", "QueueName": "one-too-many"}"#.to_owned());
    let input = lines.join("\n");

    let responses = run(&input);

    assert_eq!(responses.len(), 1001);
    assert!(responses[..1000].iter().all(|r| r == &json!({"ok": true})));
    assert_error(&responses[1000], "LimitExceeded");
}

#[test]
fn get_queue_attributes_shows_defaults_and_zero_counts() {
    let response = last(&[
        r#"{"Command": "CreateQueue", "QueueName": "jobs"}"#,
        r#"{"Command": "GetQueueAttributes", "QueueName": "jobs"}"#,
    ]);

    assert_eq!(
        response,
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
fn get_queue_attributes_shows_dead_letter_members_only_when_set() {
    let responses = session(&[
        r#"{"Command": "CreateQueue", "QueueName": "dlq"}"#,
        r#"{"Command": "CreateQueue", "QueueName": "jobs", "Attributes": {"DeadLetterQueue": "dlq", "MaxReceiveCount": 4}}"#,
        r#"{"Command": "GetQueueAttributes", "QueueName": "jobs"}"#,
    ]);

    assert_eq!(responses[2]["Attributes"]["DeadLetterQueue"], "dlq");
    assert_eq!(responses[2]["Attributes"]["MaxReceiveCount"], 4);
}

#[test]
fn get_queue_attributes_on_missing_queue_fails() {
    let response = last(&[r#"{"Command": "GetQueueAttributes", "QueueName": "nope"}"#]);

    assert_error(&response, "QueueDoesNotExist");
}

#[test]
fn list_queues_sorts_by_byte_value_and_filters_by_prefix() {
    let responses = session(&[
        r#"{"Command": "CreateQueue", "QueueName": "b"}"#,
        r#"{"Command": "CreateQueue", "QueueName": "A"}"#,
        r#"{"Command": "CreateQueue", "QueueName": "a"}"#,
        r#"{"Command": "CreateQueue", "QueueName": "_x"}"#,
        r#"{"Command": "CreateQueue", "QueueName": "-x"}"#,
        r#"{"Command": "CreateQueue", "QueueName": "1"}"#,
        r#"{"Command": "ListQueues"}"#,
        r#"{"Command": "ListQueues", "Prefix": "a"}"#,
        r#"{"Command": "ListQueues", "Prefix": ""}"#,
        r#"{"Command": "ListQueues", "Prefix": null}"#,
        r#"{"Command": "ListQueues", "Prefix": "z"}"#,
    ]);

    assert_eq!(
        responses[6],
        json!({"ok": true, "QueueNames": ["-x", "1", "A", "_x", "a", "b"]})
    );
    assert_eq!(responses[7], json!({"ok": true, "QueueNames": ["a"]}));
    assert_eq!(
        responses[8],
        json!({"ok": true, "QueueNames": ["-x", "1", "A", "_x", "a", "b"]})
    );
    assert_eq!(
        responses[9],
        json!({"ok": true, "QueueNames": ["-x", "1", "A", "_x", "a", "b"]})
    );
    assert_eq!(responses[10], json!({"ok": true, "QueueNames": []}));
}

#[test]
fn list_queues_prefix_must_be_a_string() {
    let response = last(&[r#"{"Command": "ListQueues", "Prefix": 1}"#]);

    assert_error(&response, "InvalidParameterValue");
}

#[test]
fn delete_queue_removes_it_and_missing_queue_fails() {
    let responses = session(&[
        r#"{"Command": "CreateQueue", "QueueName": "jobs"}"#,
        r#"{"Command": "DeleteQueue", "QueueName": "jobs"}"#,
        r#"{"Command": "DeleteQueue", "QueueName": "jobs"}"#,
        r#"{"Command": "GetQueueAttributes", "QueueName": "jobs"}"#,
    ]);

    assert_ok(&responses[1]);
    assert_error(&responses[2], "QueueDoesNotExist");
    assert_error(&responses[3], "QueueDoesNotExist");
}

#[test]
fn deleting_a_dead_letter_queue_clears_the_reference_and_a_new_queue_starts_fresh() {
    let responses = session(&[
        r#"{"Command": "CreateQueue", "QueueName": "dlq"}"#,
        r#"{"Command": "CreateQueue", "QueueName": "jobs", "Attributes": {"DeadLetterQueue": "dlq", "MaxReceiveCount": 4}}"#,
        r#"{"Command": "DeleteQueue", "QueueName": "dlq"}"#,
        r#"{"Command": "GetQueueAttributes", "QueueName": "jobs"}"#,
        r#"{"Command": "CreateQueue", "QueueName": "dlq"}"#,
        r#"{"Command": "GetQueueAttributes", "QueueName": "jobs"}"#,
    ]);

    assert_ok(&responses[2]);
    assert!(responses[3]["Attributes"].get("DeadLetterQueue").is_none());
    assert!(responses[3]["Attributes"].get("MaxReceiveCount").is_none());
    assert_ok(&responses[4]);
    assert!(responses[5]["Attributes"].get("DeadLetterQueue").is_none());
}

#[test]
fn set_queue_attributes_changes_only_the_attributes_given() {
    let responses = session(&[
        r#"{"Command": "CreateQueue", "QueueName": "jobs"}"#,
        r#"{"Command": "SetQueueAttributes", "QueueName": "jobs", "Attributes": {"VisibilityTimeout": 60}}"#,
        r#"{"Command": "GetQueueAttributes", "QueueName": "jobs"}"#,
    ]);

    assert_ok(&responses[1]);
    assert_eq!(responses[2]["Attributes"]["VisibilityTimeout"], 60);
    assert_eq!(responses[2]["Attributes"]["DelaySeconds"], 0);
    assert_eq!(responses[2]["Attributes"]["RetentionSeconds"], 345600);
}

#[test]
fn set_queue_attributes_accepts_an_empty_attributes_object() {
    let responses = session(&[
        r#"{"Command": "CreateQueue", "QueueName": "jobs"}"#,
        r#"{"Command": "SetQueueAttributes", "QueueName": "jobs", "Attributes": {}}"#,
    ]);

    assert_ok(&responses[1]);
}

#[test]
fn set_queue_attributes_requires_queue_and_attributes_parameters() {
    let responses = session(&[
        r#"{"Command": "CreateQueue", "QueueName": "jobs"}"#,
        r#"{"Command": "SetQueueAttributes", "QueueName": "jobs"}"#,
        r#"{"Command": "SetQueueAttributes", "Attributes": {}}"#,
        r#"{"Command": "SetQueueAttributes", "QueueName": "jobs", "Attributes": null}"#,
    ]);

    assert_error(&responses[1], "MissingParameter");
    assert_error(&responses[2], "MissingParameter");
    assert_error(&responses[3], "MissingParameter");
}

#[test]
fn set_queue_attributes_refuses_to_change_the_fifo_type() {
    let responses = session(&[
        r#"{"Command": "CreateQueue", "QueueName": "jobs"}"#,
        r#"{"Command": "SetQueueAttributes", "QueueName": "jobs", "Attributes": {"Fifo": false}}"#,
        r#"{"Command": "SetQueueAttributes", "QueueName": "missing", "Attributes": {"Fifo": true}}"#,
    ]);

    assert_error(&responses[1], "UnsupportedOperation");
    assert_error(&responses[2], "QueueDoesNotExist");
}

#[test]
fn set_queue_attributes_rejects_content_based_deduplication_on_a_standard_queue() {
    let responses = session(&[
        r#"{"Command": "CreateQueue", "QueueName": "jobs"}"#,
        r#"{"Command": "SetQueueAttributes", "QueueName": "jobs", "Attributes": {"ContentBasedDeduplication": true}}"#,
        r#"{"Command": "SetQueueAttributes", "QueueName": "jobs", "Attributes": {"ContentBasedDeduplication": false}}"#,
    ]);

    assert_error(&responses[1], "InvalidParameterValue");
    assert_ok(&responses[2]);
}

#[test]
fn set_queue_attributes_takes_dead_letter_members_together_and_can_replace_them() {
    let responses = session(&[
        r#"{"Command": "CreateQueue", "QueueName": "dlq1"}"#,
        r#"{"Command": "CreateQueue", "QueueName": "dlq2"}"#,
        r#"{"Command": "CreateQueue", "QueueName": "jobs", "Attributes": {"DeadLetterQueue": "dlq1", "MaxReceiveCount": 2}}"#,
        r#"{"Command": "SetQueueAttributes", "QueueName": "jobs", "Attributes": {"DeadLetterQueue": "dlq2"}}"#,
        r#"{"Command": "SetQueueAttributes", "QueueName": "jobs", "Attributes": {"DeadLetterQueue": "dlq2", "MaxReceiveCount": 5}}"#,
        r#"{"Command": "GetQueueAttributes", "QueueName": "jobs"}"#,
    ]);

    assert_error(&responses[3], "InvalidParameterValue");
    assert_ok(&responses[4]);
    assert_eq!(responses[5]["Attributes"]["DeadLetterQueue"], "dlq2");
    assert_eq!(responses[5]["Attributes"]["MaxReceiveCount"], 5);
}

#[test]
fn set_queue_attributes_rejects_a_dead_letter_queue_that_is_missing_itself_or_mismatched() {
    let responses = session(&[
        r#"{"Command": "CreateQueue", "QueueName": "jobs"}"#,
        r#"{"Command": "CreateQueue", "QueueName": "f.fifo", "Attributes": {"Fifo": true}}"#,
        r#"{"Command": "SetQueueAttributes", "QueueName": "jobs", "Attributes": {"DeadLetterQueue": "nope", "MaxReceiveCount": 2}}"#,
        r#"{"Command": "SetQueueAttributes", "QueueName": "jobs", "Attributes": {"DeadLetterQueue": "jobs", "MaxReceiveCount": 2}}"#,
        r#"{"Command": "SetQueueAttributes", "QueueName": "jobs", "Attributes": {"DeadLetterQueue": "f.fifo", "MaxReceiveCount": 2}}"#,
    ]);

    assert_error(&responses[2], "QueueDoesNotExist");
    assert_error(&responses[3], "InvalidParameterValue");
    assert_error(&responses[4], "InvalidParameterValue");
}

#[test]
fn tick_advances_the_clock_and_reports_it() {
    let responses = session(&[
        r#"{"Command": "Tick", "Seconds": 5}"#,
        r#"{"Command": "Tick", "Seconds": 10}"#,
        r#"{"Command": "Tick", "Seconds": 0}"#,
        r#"{"Command": "Tick", "Seconds": 1209600}"#,
    ]);

    assert_eq!(responses[0], json!({"ok": true, "Now": 5}));
    assert_eq!(responses[1], json!({"ok": true, "Now": 15}));
    assert_eq!(responses[2], json!({"ok": true, "Now": 15}));
    assert_eq!(responses[3], json!({"ok": true, "Now": 1_209_615}));
}

#[test]
fn tick_seconds_are_required_and_bounded() {
    let responses = session(&[
        r#"{"Command": "Tick"}"#,
        r#"{"Command": "Tick", "Seconds": 1209601}"#,
        r#"{"Command": "Tick", "Seconds": "5"}"#,
        r#"{"Command": "Tick", "Seconds": 1.5}"#,
    ]);

    assert_error(&responses[0], "MissingParameter");
    assert_error(&responses[1], "InvalidParameterValue");
    assert_error(&responses[2], "InvalidParameterValue");
    assert_error(&responses[3], "InvalidParameterValue");
}
