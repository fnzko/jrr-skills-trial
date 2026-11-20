mod common;

use common::{error_code, run};
use serde_json::json;

#[test]
fn create_then_get_returns_defaults_and_zero_counts() {
    let out = run(&[
        r#"{"Command": "CreateQueue", "QueueName": "jobs"}"#,
        r#"{"Command": "GetQueueAttributes", "QueueName": "jobs"}"#,
    ]);
    assert_eq!(out[0], json!({"ok": true}));
    assert_eq!(
        out[1],
        json!({
            "ok": true,
            "Attributes": {
                "VisibilityTimeout": 30,
                "DelaySeconds": 0,
                "RetentionSeconds": 345600,
                "Fifo": false,
                "ContentBasedDeduplication": false
            },
            "Counts": {"Visible": 0, "InFlight": 0, "Delayed": 0}
        })
    );
}

#[test]
fn queue_name_syntax() {
    let long = "a".repeat(80);
    let too_long = "a".repeat(81);
    let long_fifo = format!("{}.fifo", "a".repeat(75));
    let too_long_fifo = format!("{}.fifo", "a".repeat(76));
    let cases = [
        ("jobs", true),
        ("A-z_09", true),
        (long.as_str(), true),
        (too_long.as_str(), false),
        ("", false),
        ("a b", false),
        ("a.b", false),
        ("é", false),
        (".fifo", false),
        (long_fifo.as_str(), true),
        (too_long_fifo.as_str(), false),
    ];
    for (name, valid) in cases {
        let fifo = name.ends_with(".fifo");
        let command =
            json!({"Command": "CreateQueue", "QueueName": name, "Attributes": {"Fifo": fifo}});
        let out = run(&[&command.to_string()]);
        if valid {
            assert_eq!(out[0], json!({"ok": true}), "{name:?}");
        } else {
            assert_eq!(error_code(&out[0]), "QueueNameInvalid", "{name:?}");
        }
    }
}

#[test]
fn fifo_suffix_and_attribute_must_agree() {
    let out = run(&[
        r#"{"Command": "CreateQueue", "QueueName": "a.fifo"}"#,
        r#"{"Command": "CreateQueue", "QueueName": "b", "Attributes": {"Fifo": true}}"#,
        r#"{"Command": "CreateQueue", "QueueName": "c.fifo", "Attributes": {"Fifo": true, "ContentBasedDeduplication": true}}"#,
        r#"{"Command": "GetQueueAttributes", "QueueName": "c.fifo"}"#,
    ]);
    assert_eq!(error_code(&out[0]), "QueueNameInvalid");
    assert_eq!(error_code(&out[1]), "QueueNameInvalid");
    assert_eq!(out[2], json!({"ok": true}));
    assert_eq!(out[3]["Attributes"]["Fifo"], true);
    assert_eq!(out[3]["Attributes"]["ContentBasedDeduplication"], true);
}

#[test]
fn create_checks_attribute_values_in_the_documented_order() {
    let out = run(&[
        // Missing parameter beats everything.
        r#"{"Command": "CreateQueue", "Attributes": 5}"#,
        // The name is checked before the attributes.
        r#"{"Command": "CreateQueue", "QueueName": "bad name", "Attributes": 5}"#,
        r#"{"Command": "CreateQueue", "QueueName": 5}"#,
        r#"{"Command": "CreateQueue", "QueueName": "q", "Attributes": 5}"#,
        r#"{"Command": "CreateQueue", "QueueName": "q", "Attributes": {"Nope": 1}}"#,
        r#"{"Command": "CreateQueue", "QueueName": "q", "Attributes": {"VisibilityTimeout": 43201}}"#,
        r#"{"Command": "CreateQueue", "QueueName": "q", "Attributes": {"DelaySeconds": 901}}"#,
        r#"{"Command": "CreateQueue", "QueueName": "q", "Attributes": {"RetentionSeconds": 59}}"#,
        r#"{"Command": "CreateQueue", "QueueName": "q", "Attributes": {"Fifo": "yes"}}"#,
        r#"{"Command": "CreateQueue", "QueueName": "q", "Attributes": {"MaxReceiveCount": 0, "DeadLetterQueue": "d"}}"#,
        r#"{"Command": "CreateQueue", "QueueName": "q", "Attributes": {"ContentBasedDeduplication": true}}"#,
        r#"{"Command": "CreateQueue", "QueueName": "q", "Attributes": {"MaxReceiveCount": 3}}"#,
        r#"{"Command": "CreateQueue", "QueueName": "q", "Attributes": {"DeadLetterQueue": "d", "MaxReceiveCount": 3}}"#,
        r#"{"Command": "CreateQueue", "QueueName": "q", "Attributes": {"Fifo": null, "VisibilityTimeout": null}}"#,
    ]);
    let codes: Vec<_> = out.iter().map(error_code_or_ok).collect();
    assert_eq!(
        codes,
        [
            "MissingParameter",
            "QueueNameInvalid",
            "InvalidParameterValue",
            "InvalidParameterValue",
            "InvalidParameterValue",
            "InvalidParameterValue",
            "InvalidParameterValue",
            "InvalidParameterValue",
            "InvalidParameterValue",
            "InvalidParameterValue",
            "InvalidParameterValue",
            "InvalidParameterValue",
            "QueueDoesNotExist",
            "ok",
        ]
    );
}

fn error_code_or_ok(response: &serde_json::Value) -> &str {
    if response["ok"] == true {
        "ok"
    } else {
        error_code(response)
    }
}

#[test]
fn create_is_idempotent_only_for_identical_attributes() {
    let out = run(&[
        r#"{"Command": "CreateQueue", "QueueName": "q", "Attributes": {"VisibilityTimeout": 10}}"#,
        r#"{"Command": "CreateQueue", "QueueName": "q", "Attributes": {"VisibilityTimeout": 10, "DelaySeconds": 0}}"#,
        r#"{"Command": "CreateQueue", "QueueName": "q"}"#,
        r#"{"Command": "CreateQueue", "QueueName": "q", "Attributes": {"VisibilityTimeout": 11}}"#,
        r#"{"Command": "CreateQueue", "QueueName": "q.fifo", "Attributes": {"Fifo": true}}"#,
    ]);
    assert_eq!(out[0], json!({"ok": true}));
    assert_eq!(out[1], json!({"ok": true}));
    assert_eq!(error_code(&out[2]), "QueueAlreadyExists");
    assert_eq!(error_code(&out[3]), "QueueAlreadyExists");
    assert_eq!(out[4], json!({"ok": true}));
}

#[test]
fn dead_letter_configuration_is_validated_on_create() {
    let out = run(&[
        r#"{"Command": "CreateQueue", "QueueName": "dlq"}"#,
        r#"{"Command": "CreateQueue", "QueueName": "dlq.fifo", "Attributes": {"Fifo": true}}"#,
        r#"{"Command": "CreateQueue", "QueueName": "a", "Attributes": {"DeadLetterQueue": "dlq", "MaxReceiveCount": 2}}"#,
        r#"{"Command": "CreateQueue", "QueueName": "b", "Attributes": {"DeadLetterQueue": "dlq.fifo", "MaxReceiveCount": 2}}"#,
        r#"{"Command": "CreateQueue", "QueueName": "c", "Attributes": {"DeadLetterQueue": "missing", "MaxReceiveCount": 2}}"#,
        r#"{"Command": "GetQueueAttributes", "QueueName": "a"}"#,
        // An identical repeat succeeds without looking at the dead-letter queue again.
        r#"{"Command": "CreateQueue", "QueueName": "a", "Attributes": {"DeadLetterQueue": "dlq", "MaxReceiveCount": 2}}"#,
    ]);
    assert_eq!(out[2], json!({"ok": true}));
    assert_eq!(error_code(&out[3]), "InvalidParameterValue");
    assert_eq!(error_code(&out[4]), "QueueDoesNotExist");
    assert_eq!(out[5]["Attributes"]["DeadLetterQueue"], "dlq");
    assert_eq!(out[5]["Attributes"]["MaxReceiveCount"], 2);
    assert_eq!(out[6], json!({"ok": true}));
}

#[test]
fn at_most_1000_queues_exist() {
    let mut commands: Vec<String> = (0..1000)
        .map(|i| format!(r#"{{"Command": "CreateQueue", "QueueName": "q{i}"}}"#))
        .collect();
    commands.push(r#"{"Command": "CreateQueue", "QueueName": "one-too-many"}"#.into());
    commands.push(r#"{"Command": "CreateQueue", "QueueName": "q7"}"#.into());
    commands.push(r#"{"Command": "DeleteQueue", "QueueName": "q7"}"#.into());
    commands.push(r#"{"Command": "CreateQueue", "QueueName": "one-more"}"#.into());
    let refs: Vec<&str> = commands.iter().map(String::as_str).collect();
    let out = run(&refs);
    assert!(out[..1000].iter().all(|r| r["ok"] == true));
    assert_eq!(error_code(&out[1000]), "LimitExceeded");
    assert_eq!(out[1001], json!({"ok": true}));
    assert_eq!(out[1002], json!({"ok": true}));
    assert_eq!(out[1003], json!({"ok": true}));
}

#[test]
fn set_queue_attributes_updates_only_the_given_attributes() {
    let out = run(&[
        r#"{"Command": "CreateQueue", "QueueName": "q", "Attributes": {"DelaySeconds": 5}}"#,
        r#"{"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"VisibilityTimeout": 99, "RetentionSeconds": null}}"#,
        r#"{"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {}}"#,
        r#"{"Command": "GetQueueAttributes", "QueueName": "q"}"#,
    ]);
    assert_eq!(out[1], json!({"ok": true}));
    assert_eq!(out[2], json!({"ok": true}));
    assert_eq!(
        out[3]["Attributes"],
        json!({
            "VisibilityTimeout": 99,
            "DelaySeconds": 5,
            "RetentionSeconds": 345600,
            "Fifo": false,
            "ContentBasedDeduplication": false
        })
    );
}

#[test]
fn set_queue_attributes_rules_in_order() {
    let out = run(&[
        r#"{"Command": "CreateQueue", "QueueName": "std"}"#,
        r#"{"Command": "CreateQueue", "QueueName": "std2"}"#,
        r#"{"Command": "CreateQueue", "QueueName": "f.fifo", "Attributes": {"Fifo": true}}"#,
        r#"{"Command": "SetQueueAttributes", "QueueName": "std"}"#,
        r#"{"Command": "SetQueueAttributes", "QueueName": "nope", "Attributes": {"Fifo": true}}"#,
        r#"{"Command": "SetQueueAttributes", "QueueName": "std", "Attributes": {"Fifo": false}}"#,
        r#"{"Command": "SetQueueAttributes", "QueueName": "std", "Attributes": {"ContentBasedDeduplication": true}}"#,
        r#"{"Command": "SetQueueAttributes", "QueueName": "std", "Attributes": {"MaxReceiveCount": 2}}"#,
        r#"{"Command": "SetQueueAttributes", "QueueName": "std", "Attributes": {"DeadLetterQueue": "gone", "MaxReceiveCount": 2}}"#,
        r#"{"Command": "SetQueueAttributes", "QueueName": "std", "Attributes": {"DeadLetterQueue": "std", "MaxReceiveCount": 2}}"#,
        r#"{"Command": "SetQueueAttributes", "QueueName": "std", "Attributes": {"DeadLetterQueue": "f.fifo", "MaxReceiveCount": 2}}"#,
        r#"{"Command": "SetQueueAttributes", "QueueName": "std", "Attributes": {"DeadLetterQueue": "std2", "MaxReceiveCount": 2}}"#,
        r#"{"Command": "SetQueueAttributes", "QueueName": "std", "Attributes": {"VisibilityTimeout": 1}}"#,
        r#"{"Command": "GetQueueAttributes", "QueueName": "std"}"#,
    ]);
    let codes: Vec<_> = out[3..].iter().map(error_code_or_ok).collect();
    assert_eq!(
        codes,
        [
            "MissingParameter",
            "QueueDoesNotExist",
            "UnsupportedOperation",
            "InvalidParameterValue",
            "InvalidParameterValue",
            "QueueDoesNotExist",
            "InvalidParameterValue",
            "InvalidParameterValue",
            "ok",
            "ok",
            "ok",
        ]
    );
    // A dead-letter configuration is kept when other attributes change.
    assert_eq!(out[13]["Attributes"]["DeadLetterQueue"], "std2");
    assert_eq!(out[13]["Attributes"]["MaxReceiveCount"], 2);
}

#[test]
fn list_queues_filters_by_prefix_and_sorts_by_bytes() {
    let names = ["b", "a", "_x", "Z", "z", "1", "a-b", "a_b", "a.fifo"];
    let mut commands: Vec<String> = names
        .iter()
        .map(|n| {
            let fifo = n.ends_with(".fifo");
            format!(r#"{{"Command": "CreateQueue", "QueueName": "{n}", "Attributes": {{"Fifo": {fifo}}}}}"#)
        })
        .collect();
    commands.push(r#"{"Command": "ListQueues"}"#.into());
    commands.push(r#"{"Command": "ListQueues", "Prefix": "a"}"#.into());
    commands.push(r#"{"Command": "ListQueues", "Prefix": ""}"#.into());
    commands.push(r#"{"Command": "ListQueues", "Prefix": "nothing"}"#.into());
    commands.push(r#"{"Command": "ListQueues", "Prefix": 1}"#.into());
    let refs: Vec<&str> = commands.iter().map(String::as_str).collect();
    let out = run(&refs);
    // Byte order: digits < uppercase < '_' < lowercase, and '-' < '.' < '_' after "a".
    let expected_all = json!(["1", "Z", "_x", "a", "a-b", "a.fifo", "a_b", "b", "z"]);
    assert_eq!(out[9], json!({"ok": true, "QueueNames": expected_all}));
    assert_eq!(
        out[10],
        json!({"ok": true, "QueueNames": ["a", "a-b", "a.fifo", "a_b"]})
    );
    assert_eq!(out[11], out[9]);
    assert_eq!(out[12], json!({"ok": true, "QueueNames": []}));
    assert_eq!(error_code(&out[13]), "InvalidParameterValue");
}

#[test]
fn delete_queue_clears_dead_letter_configuration_of_referrers() {
    let out = run(&[
        r#"{"Command": "CreateQueue", "QueueName": "dlq"}"#,
        r#"{"Command": "CreateQueue", "QueueName": "q", "Attributes": {"DeadLetterQueue": "dlq", "MaxReceiveCount": 2}}"#,
        r#"{"Command": "DeleteQueue", "QueueName": "dlq"}"#,
        r#"{"Command": "GetQueueAttributes", "QueueName": "q"}"#,
        r#"{"Command": "DeleteQueue", "QueueName": "dlq"}"#,
        r#"{"Command": "GetQueueAttributes", "QueueName": "dlq"}"#,
        r#"{"Command": "CreateQueue", "QueueName": "q", "Attributes": {"DeadLetterQueue": "dlq", "MaxReceiveCount": 2}}"#,
    ]);
    assert_eq!(out[2], json!({"ok": true}));
    assert_eq!(out[3]["Attributes"].get("DeadLetterQueue"), None);
    assert_eq!(out[3]["Attributes"].get("MaxReceiveCount"), None);
    assert_eq!(error_code(&out[4]), "QueueDoesNotExist");
    assert_eq!(error_code(&out[5]), "QueueDoesNotExist");
    assert_eq!(error_code(&out[6]), "QueueAlreadyExists");
}

#[test]
fn queue_lookup_uses_the_exact_name_whatever_it_looks_like() {
    let out = run(&[
        r#"{"Command": "GetQueueAttributes", "QueueName": "bad name!"}"#,
        r#"{"Command": "PurgeQueue", "QueueName": ""}"#,
        r#"{"Command": "DeleteQueue", "QueueName": "Q"}"#,
        r#"{"Command": "PurgeQueue"}"#,
        r#"{"Command": "PurgeQueue", "QueueName": 3}"#,
    ]);
    assert_eq!(error_code(&out[0]), "QueueDoesNotExist");
    assert_eq!(error_code(&out[1]), "QueueDoesNotExist");
    assert_eq!(error_code(&out[2]), "QueueDoesNotExist");
    assert_eq!(error_code(&out[3]), "MissingParameter");
    assert_eq!(error_code(&out[4]), "InvalidParameterValue");
}
