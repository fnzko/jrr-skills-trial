use crate::common::{error_code, session};
use serde_json::json;

fn defaults() -> serde_json::Value {
    json!({
        "VisibilityTimeout": 30,
        "DelaySeconds": 0,
        "RetentionSeconds": 345_600,
        "Fifo": false,
        "ContentBasedDeduplication": false,
    })
}

#[test]
fn create_then_get_returns_defaults_and_zero_counts() {
    let responses = session(&[
        r#"{"Command": "CreateQueue", "QueueName": "jobs"}"#,
        r#"{"Command": "GetQueueAttributes", "QueueName": "jobs"}"#,
    ]);
    assert_eq!(responses[0], json!({"ok": true}));
    assert_eq!(
        responses[1],
        json!({
            "ok": true,
            "Attributes": defaults(),
            "Counts": {"Visible": 0, "InFlight": 0, "Delayed": 0},
        })
    );
}

#[test]
fn create_is_idempotent_only_for_identical_attributes() {
    let responses = session(&[
        r#"{"Command": "CreateQueue", "QueueName": "jobs", "Attributes": {"VisibilityTimeout": 5}}"#,
        r#"{"Command": "CreateQueue", "QueueName": "jobs", "Attributes": {"VisibilityTimeout": 5, "DelaySeconds": null}}"#,
        r#"{"Command": "CreateQueue", "QueueName": "jobs"}"#,
    ]);
    assert_eq!(responses[1], json!({"ok": true}));
    assert_eq!(error_code(&responses[2]), Some("QueueAlreadyExists"));
}

#[test]
fn create_rules_report_the_first_failing_check() {
    let cases = [
        (
            r#"{"Command": "CreateQueue", "QueueName": "bad name"}"#,
            "QueueNameInvalid",
        ),
        (
            r#"{"Command": "CreateQueue", "QueueName": "bad name", "Attributes": 5}"#,
            "QueueNameInvalid",
        ),
        (
            r#"{"Command": "CreateQueue", "QueueName": 5}"#,
            "InvalidParameterValue",
        ),
        (r#"{"Command": "CreateQueue"}"#, "MissingParameter"),
        (
            r#"{"Command": "CreateQueue", "QueueName": "a", "Attributes": 5}"#,
            "InvalidParameterValue",
        ),
        (
            r#"{"Command": "CreateQueue", "QueueName": "a", "Attributes": {"Nope": 1}}"#,
            "InvalidParameterValue",
        ),
        (
            r#"{"Command": "CreateQueue", "QueueName": "a.fifo"}"#,
            "QueueNameInvalid",
        ),
        (
            r#"{"Command": "CreateQueue", "QueueName": "a", "Attributes": {"Fifo": true}}"#,
            "QueueNameInvalid",
        ),
        (
            r#"{"Command": "CreateQueue", "QueueName": "a", "Attributes": {"ContentBasedDeduplication": true}}"#,
            "InvalidParameterValue",
        ),
        (
            r#"{"Command": "CreateQueue", "QueueName": "a", "Attributes": {"MaxReceiveCount": 2}}"#,
            "InvalidParameterValue",
        ),
        (
            r#"{"Command": "CreateQueue", "QueueName": "a", "Attributes": {"DeadLetterQueue": "x", "MaxReceiveCount": 2}}"#,
            "QueueDoesNotExist",
        ),
    ];
    for (command, expected) in cases {
        let responses = session(&[command]);
        assert_eq!(error_code(&responses[0]), Some(expected), "{command}");
    }
}

#[test]
fn dead_letter_queue_must_have_the_same_type() {
    let responses = session(&[
        r#"{"Command": "CreateQueue", "QueueName": "dead"}"#,
        r#"{"Command": "CreateQueue", "QueueName": "a.fifo", "Attributes": {"Fifo": true, "DeadLetterQueue": "dead", "MaxReceiveCount": 2}}"#,
        r#"{"Command": "CreateQueue", "QueueName": "a", "Attributes": {"DeadLetterQueue": "dead", "MaxReceiveCount": 2}}"#,
        r#"{"Command": "GetQueueAttributes", "QueueName": "a"}"#,
        r#"{"Command": "CreateQueue", "QueueName": "a", "Attributes": {"DeadLetterQueue": "dead", "MaxReceiveCount": 2}}"#,
        r#"{"Command": "CreateQueue", "QueueName": "a", "Attributes": {"DeadLetterQueue": "dead", "MaxReceiveCount": 3}}"#,
    ]);
    assert_eq!(error_code(&responses[1]), Some("InvalidParameterValue"));
    assert_eq!(responses[3]["Attributes"]["DeadLetterQueue"], json!("dead"));
    assert_eq!(responses[3]["Attributes"]["MaxReceiveCount"], json!(2));
    assert_eq!(responses[4], json!({"ok": true}));
    assert_eq!(error_code(&responses[5]), Some("QueueAlreadyExists"));
}

#[test]
fn at_most_1000_queues_can_exist() {
    let mut commands: Vec<String> = (0..1001)
        .map(|n| format!(r#"{{"Command": "CreateQueue", "QueueName": "q{n}"}}"#))
        .collect();
    commands.push(r#"{"Command": "CreateQueue", "QueueName": "q0"}"#.to_owned());
    let refs: Vec<&str> = commands.iter().map(String::as_str).collect();
    let responses = session(&refs);
    assert_eq!(error_code(&responses[1000]), Some("LimitExceeded"));
    assert_eq!(responses[999], json!({"ok": true}));
    assert_eq!(responses[1001], json!({"ok": true}));
}

#[test]
fn list_queues_filters_by_prefix_in_byte_order() {
    let responses = session(&[
        r#"{"Command": "CreateQueue", "QueueName": "b"}"#,
        r#"{"Command": "CreateQueue", "QueueName": "_x"}"#,
        r#"{"Command": "CreateQueue", "QueueName": "Z"}"#,
        r#"{"Command": "CreateQueue", "QueueName": "a-1"}"#,
        r#"{"Command": "CreateQueue", "QueueName": "a1"}"#,
        r#"{"Command": "CreateQueue", "QueueName": "a.fifo", "Attributes": {"Fifo": true}}"#,
        r#"{"Command": "ListQueues"}"#,
        r#"{"Command": "ListQueues", "Prefix": "a"}"#,
        r#"{"Command": "ListQueues", "Prefix": "nothing"}"#,
        r#"{"Command": "ListQueues", "Prefix": 1}"#,
    ]);
    assert_eq!(
        responses[6],
        json!({"ok": true, "QueueNames": ["Z", "_x", "a-1", "a.fifo", "a1", "b"]})
    );
    assert_eq!(
        responses[7],
        json!({"ok": true, "QueueNames": ["a-1", "a.fifo", "a1"]})
    );
    assert_eq!(responses[8], json!({"ok": true, "QueueNames": []}));
    assert_eq!(error_code(&responses[9]), Some("InvalidParameterValue"));
}

#[test]
fn set_attributes_rules_in_order() {
    let responses = session(&[
        r#"{"Command": "CreateQueue", "QueueName": "a"}"#,
        r#"{"Command": "CreateQueue", "QueueName": "f.fifo", "Attributes": {"Fifo": true}}"#,
        r#"{"Command": "SetQueueAttributes", "QueueName": "a"}"#,
        r#"{"Command": "SetQueueAttributes", "QueueName": "nope", "Attributes": {"Fifo": 1}}"#,
        r#"{"Command": "SetQueueAttributes", "QueueName": "a", "Attributes": {"Fifo": true}}"#,
        r#"{"Command": "SetQueueAttributes", "QueueName": "a", "Attributes": {"ContentBasedDeduplication": true}}"#,
        r#"{"Command": "SetQueueAttributes", "QueueName": "a", "Attributes": {"MaxReceiveCount": 3}}"#,
        r#"{"Command": "SetQueueAttributes", "QueueName": "a", "Attributes": {"DeadLetterQueue": "nope", "MaxReceiveCount": 3}}"#,
        r#"{"Command": "SetQueueAttributes", "QueueName": "a", "Attributes": {"DeadLetterQueue": "a", "MaxReceiveCount": 3}}"#,
        r#"{"Command": "SetQueueAttributes", "QueueName": "a", "Attributes": {"DeadLetterQueue": "f.fifo", "MaxReceiveCount": 3}}"#,
        r#"{"Command": "SetQueueAttributes", "QueueName": "f.fifo", "Attributes": {"ContentBasedDeduplication": true, "DelaySeconds": 7}}"#,
        r#"{"Command": "GetQueueAttributes", "QueueName": "f.fifo"}"#,
    ]);
    let codes: Vec<_> = responses[2..10].iter().map(error_code).collect();
    assert_eq!(
        codes,
        [
            Some("MissingParameter"),
            Some("InvalidParameterValue"),
            Some("UnsupportedOperation"),
            Some("InvalidParameterValue"),
            Some("InvalidParameterValue"),
            Some("QueueDoesNotExist"),
            Some("InvalidParameterValue"),
            Some("InvalidParameterValue"),
        ]
    );
    assert_eq!(responses[10], json!({"ok": true}));
    assert_eq!(
        responses[11]["Attributes"],
        json!({
            "VisibilityTimeout": 30,
            "DelaySeconds": 7,
            "RetentionSeconds": 345_600,
            "Fifo": true,
            "ContentBasedDeduplication": true,
        })
    );
}

#[test]
fn deleting_a_dead_letter_queue_clears_the_configuration_of_its_users() {
    let responses = session(&[
        r#"{"Command": "CreateQueue", "QueueName": "dead"}"#,
        r#"{"Command": "CreateQueue", "QueueName": "a", "Attributes": {"DeadLetterQueue": "dead", "MaxReceiveCount": 2}}"#,
        r#"{"Command": "DeleteQueue", "QueueName": "dead"}"#,
        r#"{"Command": "GetQueueAttributes", "QueueName": "a"}"#,
        r#"{"Command": "DeleteQueue", "QueueName": "dead"}"#,
    ]);
    assert_eq!(responses[3]["Attributes"], defaults());
    assert_eq!(error_code(&responses[4]), Some("QueueDoesNotExist"));
}

#[test]
fn tick_advances_the_clock_and_validates_seconds() {
    let responses = session(&[
        r#"{"Command": "Tick", "Seconds": 10}"#,
        r#"{"Command": "Tick", "Seconds": 0}"#,
        r#"{"Command": "Tick", "Seconds": 1209600}"#,
        r#"{"Command": "Tick", "Seconds": 1209601}"#,
        r#"{"Command": "Tick", "Seconds": 1.0}"#,
        r#"{"Command": "Tick", "Seconds": "1"}"#,
    ]);
    assert_eq!(responses[0], json!({"ok": true, "Now": 10}));
    assert_eq!(responses[1], json!({"ok": true, "Now": 10}));
    assert_eq!(responses[2], json!({"ok": true, "Now": 1_209_610}));
    for response in &responses[3..] {
        assert_eq!(error_code(response), Some("InvalidParameterValue"));
    }
}

#[test]
fn lookup_commands_report_missing_queues_whatever_the_name_looks_like() {
    let responses = session(&[
        r#"{"Command": "GetQueueAttributes", "QueueName": "no such queue!"}"#,
        r#"{"Command": "DeleteQueue", "QueueName": ""}"#,
        r#"{"Command": "PurgeQueue", "QueueName": "x"}"#,
        r#"{"Command": "PurgeQueue", "QueueName": 4}"#,
    ]);
    let codes: Vec<_> = responses.iter().map(error_code).collect();
    assert_eq!(
        codes,
        [
            Some("QueueDoesNotExist"),
            Some("QueueDoesNotExist"),
            Some("QueueDoesNotExist"),
            Some("InvalidParameterValue"),
        ]
    );
}
