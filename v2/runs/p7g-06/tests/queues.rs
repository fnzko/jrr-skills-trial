mod common;

use common::{error_code, run};
use serde_json::json;

#[test]
fn create_then_get_reports_defaults() {
    let responses = run(&[
        r#"{"Command":"CreateQueue","QueueName":"jobs"}"#,
        r#"{"Command":"GetQueueAttributes","QueueName":"jobs"}"#,
    ]);
    assert_eq!(responses[0], json!({"ok": true}));
    assert_eq!(
        responses[1],
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
fn create_is_idempotent_only_for_equal_attributes() {
    let responses = run(&[
        r#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"VisibilityTimeout":5}}"#,
        r#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"VisibilityTimeout":5,"DelaySeconds":null}}"#,
        r#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"VisibilityTimeout":6}}"#,
        r#"{"Command":"CreateQueue","QueueName":"q"}"#,
    ]);
    assert_eq!(responses[1], json!({"ok": true}));
    assert_eq!(error_code(&responses[2]), "QueueAlreadyExists");
    assert_eq!(error_code(&responses[3]), "QueueAlreadyExists");
}

#[test]
fn create_checks_follow_the_specified_order() {
    let cases = [
        (r#"{"Command":"CreateQueue"}"#, "MissingParameter"),
        (
            r#"{"Command":"CreateQueue","QueueName":"bad name","Attributes":5}"#,
            "QueueNameInvalid",
        ),
        (
            r#"{"Command":"CreateQueue","QueueName":7,"Attributes":5}"#,
            "InvalidParameterValue",
        ),
        (
            r#"{"Command":"CreateQueue","QueueName":"q","Attributes":5}"#,
            "InvalidParameterValue",
        ),
        (
            r#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"Bogus":1}}"#,
            "InvalidParameterValue",
        ),
        (
            r#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"VisibilityTimeout":43201}}"#,
            "InvalidParameterValue",
        ),
        (
            r#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"RetentionSeconds":59}}"#,
            "InvalidParameterValue",
        ),
        (
            r#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"Fifo":"yes"}}"#,
            "InvalidParameterValue",
        ),
        (
            r#"{"Command":"CreateQueue","QueueName":"q.fifo"}"#,
            "QueueNameInvalid",
        ),
        (
            r#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"Fifo":true}}"#,
            "QueueNameInvalid",
        ),
        (
            r#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"ContentBasedDeduplication":true}}"#,
            "InvalidParameterValue",
        ),
        (
            r#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"MaxReceiveCount":3}}"#,
            "InvalidParameterValue",
        ),
        (
            r#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"DeadLetterQueue":"d","MaxReceiveCount":3}}"#,
            "QueueDoesNotExist",
        ),
    ];
    for (line, code) in cases {
        assert_eq!(error_code(&run(&[line])[0]), code, "{line}");
    }
}

#[test]
fn dead_letter_queue_must_have_the_same_type() {
    let responses = run(&[
        r#"{"Command":"CreateQueue","QueueName":"d.fifo","Attributes":{"Fifo":true}}"#,
        r#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"DeadLetterQueue":"d.fifo","MaxReceiveCount":2}}"#,
        r#"{"Command":"CreateQueue","QueueName":"q2.fifo","Attributes":{"Fifo":true,"DeadLetterQueue":"d.fifo","MaxReceiveCount":2}}"#,
        r#"{"Command":"GetQueueAttributes","QueueName":"q2.fifo"}"#,
    ]);
    assert_eq!(error_code(&responses[1]), "InvalidParameterValue");
    assert_eq!(responses[2], json!({"ok": true}));
    assert_eq!(responses[3]["Attributes"]["DeadLetterQueue"], "d.fifo");
    assert_eq!(responses[3]["Attributes"]["MaxReceiveCount"], 2);
}

#[test]
fn queue_limit_is_one_thousand() {
    let mut commands: Vec<String> = (0..1001)
        .map(|n| format!(r#"{{"Command":"CreateQueue","QueueName":"q{n}"}}"#))
        .collect();
    commands.push(r#"{"Command":"CreateQueue","QueueName":"q0"}"#.to_owned());
    let refs: Vec<&str> = commands.iter().map(String::as_str).collect();
    let responses = run(&refs);
    assert_eq!(responses[999], json!({"ok": true}));
    assert_eq!(error_code(&responses[1000]), "LimitExceeded");
    assert_eq!(responses[1001], json!({"ok": true}));
}

#[test]
fn set_attributes_rules_apply_in_order() {
    let cases = [
        (
            r#"{"Command":"SetQueueAttributes","QueueName":"q"}"#,
            "MissingParameter",
        ),
        (
            r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":null}"#,
            "MissingParameter",
        ),
        (
            r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":[]}"#,
            "InvalidParameterValue",
        ),
        (
            r#"{"Command":"SetQueueAttributes","QueueName":"nope","Attributes":{"Fifo":true}}"#,
            "QueueDoesNotExist",
        ),
        (
            r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"Fifo":false}}"#,
            "UnsupportedOperation",
        ),
        (
            r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"ContentBasedDeduplication":true,"MaxReceiveCount":2}}"#,
            "InvalidParameterValue",
        ),
        (
            r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"MaxReceiveCount":2}}"#,
            "InvalidParameterValue",
        ),
        (
            r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"DeadLetterQueue":"nope","MaxReceiveCount":2}}"#,
            "QueueDoesNotExist",
        ),
        (
            r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"DeadLetterQueue":"q","MaxReceiveCount":2}}"#,
            "InvalidParameterValue",
        ),
        (
            r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"DeadLetterQueue":"f.fifo","MaxReceiveCount":2}}"#,
            "InvalidParameterValue",
        ),
    ];
    for (line, code) in cases {
        let responses = run(&[
            r#"{"Command":"CreateQueue","QueueName":"q"}"#,
            r#"{"Command":"CreateQueue","QueueName":"f.fifo","Attributes":{"Fifo":true}}"#,
            line,
        ]);
        assert_eq!(error_code(&responses[2]), code, "{line}");
    }
}

#[test]
fn set_attributes_changes_only_what_is_given_and_a_failure_changes_nothing() {
    let responses = run(&[
        r#"{"Command":"CreateQueue","QueueName":"d"}"#,
        r#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"DelaySeconds":7}}"#,
        r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"VisibilityTimeout":9,"DelaySeconds":null,"DeadLetterQueue":"d","MaxReceiveCount":4}}"#,
        r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"VisibilityTimeout":1,"RetentionSeconds":5}}"#,
        r#"{"Command":"GetQueueAttributes","QueueName":"q"}"#,
    ]);
    assert_eq!(responses[2], json!({"ok": true}));
    assert_eq!(error_code(&responses[3]), "InvalidParameterValue");
    assert_eq!(
        responses[4]["Attributes"],
        json!({
            "VisibilityTimeout": 9,
            "DelaySeconds": 7,
            "RetentionSeconds": 345600,
            "Fifo": false,
            "ContentBasedDeduplication": false,
            "DeadLetterQueue": "d",
            "MaxReceiveCount": 4
        })
    );
}

#[test]
fn list_queues_sorts_by_byte_value_and_filters_by_prefix() {
    let names = ["b", "a_", "B", "a-", "a.fifo", "a0", "aa"];
    let mut commands: Vec<String> = names
        .iter()
        .map(|name| {
            let fifo = name.ends_with(".fifo");
            format!(
                r#"{{"Command":"CreateQueue","QueueName":"{name}","Attributes":{{"Fifo":{fifo}}}}}"#
            )
        })
        .collect();
    commands.push(r#"{"Command":"ListQueues"}"#.to_owned());
    commands.push(r#"{"Command":"ListQueues","Prefix":"a"}"#.to_owned());
    commands.push(r#"{"Command":"ListQueues","Prefix":"zzz"}"#.to_owned());
    commands.push(r#"{"Command":"ListQueues","Prefix":3}"#.to_owned());
    let refs: Vec<&str> = commands.iter().map(String::as_str).collect();
    let responses = run(&refs);
    assert_eq!(
        responses[7],
        json!({"ok": true, "QueueNames": ["B", "a-", "a.fifo", "a0", "a_", "aa", "b"]})
    );
    assert_eq!(
        responses[8],
        json!({"ok": true, "QueueNames": ["a-", "a.fifo", "a0", "a_", "aa"]})
    );
    assert_eq!(responses[9], json!({"ok": true, "QueueNames": []}));
    assert_eq!(error_code(&responses[10]), "InvalidParameterValue");
}

#[test]
fn delete_queue_clears_dead_letter_configuration_of_users() {
    let responses = run(&[
        r#"{"Command":"CreateQueue","QueueName":"d"}"#,
        r#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"DeadLetterQueue":"d","MaxReceiveCount":2}}"#,
        r#"{"Command":"DeleteQueue","QueueName":"d"}"#,
        r#"{"Command":"GetQueueAttributes","QueueName":"q"}"#,
        r#"{"Command":"DeleteQueue","QueueName":"d"}"#,
        r#"{"Command":"GetQueueAttributes","QueueName":"d"}"#,
    ]);
    assert_eq!(responses[2], json!({"ok": true}));
    assert_eq!(
        responses[3]["Attributes"],
        json!({
            "VisibilityTimeout": 30,
            "DelaySeconds": 0,
            "RetentionSeconds": 345600,
            "Fifo": false,
            "ContentBasedDeduplication": false
        })
    );
    assert_eq!(error_code(&responses[4]), "QueueDoesNotExist");
    assert_eq!(error_code(&responses[5]), "QueueDoesNotExist");
}

#[test]
fn lookups_accept_any_string_and_report_a_missing_queue() {
    let responses = run(&[
        r#"{"Command":"GetQueueAttributes","QueueName":""}"#,
        r#"{"Command":"PurgeQueue","QueueName":"not a name!"}"#,
        r#"{"Command":"PurgeQueue","QueueName":5}"#,
        r#"{"Command":"PurgeQueue"}"#,
    ]);
    assert_eq!(error_code(&responses[0]), "QueueDoesNotExist");
    assert_eq!(error_code(&responses[1]), "QueueDoesNotExist");
    assert_eq!(error_code(&responses[2]), "InvalidParameterValue");
    assert_eq!(error_code(&responses[3]), "MissingParameter");
}
