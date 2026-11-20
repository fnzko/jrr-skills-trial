mod common;

use common::{error_code, run};
use serde_json::json;

fn create(name: &str, attributes: &str) -> String {
    format!(r#"{{"Command":"CreateQueue","QueueName":"{name}","Attributes":{attributes}}}"#)
}

fn set(name: &str, attributes: &str) -> String {
    format!(r#"{{"Command":"SetQueueAttributes","QueueName":"{name}","Attributes":{attributes}}}"#)
}

fn get(name: &str) -> String {
    format!(r#"{{"Command":"GetQueueAttributes","QueueName":"{name}"}}"#)
}

fn run_owned(lines: &[String]) -> Vec<serde_json::Value> {
    let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
    run(&lines)
}

#[test]
fn new_queue_has_the_default_attributes_and_no_messages() {
    let responses = run(&[
        r#"{"Command":"CreateQueue","QueueName":"jobs"}"#,
        &get("jobs"),
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
fn queue_name_must_match_its_type() {
    let responses = run_owned(&[
        create("a.fifo", "{}"),
        create("a", r#"{"Fifo":true}"#),
        create("a.fifo", r#"{"Fifo":true}"#),
        create("a", r#"{"Fifo":false}"#),
        create("bad name", "{}"),
        create(".fifo", r#"{"Fifo":true}"#),
        format!(
            r#"{{"Command":"CreateQueue","QueueName":"{}"}}"#,
            "x".repeat(81)
        ),
    ]);
    let codes: Vec<_> = responses
        .iter()
        .map(|r| if r["ok"] == true { "ok" } else { error_code(r) })
        .collect();
    assert_eq!(
        codes,
        [
            "QueueNameInvalid",
            "QueueNameInvalid",
            "ok",
            "ok",
            "QueueNameInvalid",
            "QueueNameInvalid",
            "QueueNameInvalid",
        ]
    );
}

#[test]
fn create_rules_apply_in_the_documented_order() {
    let responses = run_owned(&[
        // Rule 1 before rule 2.
        create("a", r#"{"Fifo":true,"ContentBasedDeduplication":true}"#),
        // Rule 2.
        create("a", r#"{"ContentBasedDeduplication":true}"#),
        // Rule 3 before the dead-letter queue is looked up.
        create("a", r#"{"DeadLetterQueue":"missing"}"#),
        create("a", r#"{"MaxReceiveCount":3}"#),
        // Rule 5.
        create("a", r#"{"DeadLetterQueue":"missing","MaxReceiveCount":3}"#),
        // Rule 6.
        create("std", "{}"),
        create(
            "f.fifo",
            r#"{"Fifo":true,"DeadLetterQueue":"std","MaxReceiveCount":3}"#,
        ),
        create("f.fifo", r#"{"Fifo":true}"#),
        create("s2", r#"{"DeadLetterQueue":"f.fifo","MaxReceiveCount":3}"#),
    ]);
    let codes: Vec<_> = responses
        .iter()
        .map(|r| if r["ok"] == true { "ok" } else { error_code(r) })
        .collect();
    assert_eq!(
        codes,
        [
            "QueueNameInvalid",
            "InvalidParameterValue",
            "InvalidParameterValue",
            "InvalidParameterValue",
            "QueueDoesNotExist",
            "ok",
            "InvalidParameterValue",
            "ok",
            "InvalidParameterValue",
        ]
    );
}

#[test]
fn creating_an_existing_queue_succeeds_only_with_equal_attributes() {
    let responses = run_owned(&[
        create("q", r#"{"VisibilityTimeout":10}"#),
        create("q", r#"{"VisibilityTimeout":10}"#),
        create(
            "q",
            r#"{"VisibilityTimeout":10,"DelaySeconds":0,"Fifo":false}"#,
        ),
        create("q", "{}"),
        create("q", r#"{"VisibilityTimeout":11}"#),
        // The existing queue is compared before the dead-letter queue is looked up.
        create(
            "q",
            r#"{"VisibilityTimeout":10,"DeadLetterQueue":"missing","MaxReceiveCount":2}"#,
        ),
    ]);
    assert_eq!(responses[0], json!({"ok": true}));
    assert_eq!(responses[1], json!({"ok": true}));
    assert_eq!(responses[2], json!({"ok": true}));
    assert_eq!(error_code(&responses[3]), "QueueAlreadyExists");
    assert_eq!(error_code(&responses[4]), "QueueAlreadyExists");
    assert_eq!(error_code(&responses[5]), "QueueAlreadyExists");
}

#[test]
fn recreating_a_queue_with_a_dead_letter_configuration_compares_it() {
    let dead_letter = r#"{"DeadLetterQueue":"dlq","MaxReceiveCount":2}"#;
    let responses = run_owned(&[
        create("dlq", "{}"),
        create("q", dead_letter),
        create("q", dead_letter),
        create("q", r#"{"DeadLetterQueue":"dlq","MaxReceiveCount":3}"#),
        create("q", "{}"),
    ]);
    assert_eq!(responses[2], json!({"ok": true}));
    assert_eq!(error_code(&responses[3]), "QueueAlreadyExists");
    assert_eq!(error_code(&responses[4]), "QueueAlreadyExists");
}

#[test]
fn at_most_1000_queues_can_exist() {
    let mut lines: Vec<String> = (0..1000).map(|n| create(&format!("q{n}"), "{}")).collect();
    lines.push(create("one-too-many", "{}"));
    lines.push(create("q0", "{}"));
    lines.push(r#"{"Command":"DeleteQueue","QueueName":"q0"}"#.to_owned());
    lines.push(create("one-too-many", "{}"));
    let responses = run_owned(&lines);
    assert!(responses[..1000].iter().all(|r| r["ok"] == true));
    assert_eq!(error_code(&responses[1000]), "LimitExceeded");
    // An idempotent create of an existing queue needs no room.
    assert_eq!(responses[1001], json!({"ok": true}));
    assert_eq!(responses[1002], json!({"ok": true}));
    assert_eq!(responses[1003], json!({"ok": true}));
}

#[test]
fn set_attributes_changes_only_the_given_members() {
    let responses = run_owned(&[
        create("q", r#"{"VisibilityTimeout":5,"DelaySeconds":6}"#),
        set(
            "q",
            r#"{"DelaySeconds":7,"RetentionSeconds":100,"VisibilityTimeout":null}"#,
        ),
        get("q"),
        set("q", "{}"),
    ]);
    assert_eq!(responses[1], json!({"ok": true}));
    assert_eq!(
        responses[2]["Attributes"],
        json!({
            "VisibilityTimeout": 5,
            "DelaySeconds": 7,
            "RetentionSeconds": 100,
            "Fifo": false,
            "ContentBasedDeduplication": false
        })
    );
    assert_eq!(responses[3], json!({"ok": true}));
}

#[test]
fn set_attributes_rules_apply_in_the_documented_order() {
    let responses = run_owned(&[
        create("std", "{}"),
        create("other", "{}"),
        create("f.fifo", r#"{"Fifo":true}"#),
        // The queue lookup comes first.
        set("missing", r#"{"Fifo":true}"#),
        // Rule 1 before rule 2.
        set("std", r#"{"Fifo":false,"ContentBasedDeduplication":true}"#),
        set("std", r#"{"Fifo":false}"#),
        // Rule 2.
        set("std", r#"{"ContentBasedDeduplication":true}"#),
        // Rule 3 before rule 4.
        set("std", r#"{"DeadLetterQueue":"nope"}"#),
        set("std", r#"{"MaxReceiveCount":2}"#),
        // Rule 4.
        set("std", r#"{"DeadLetterQueue":"nope","MaxReceiveCount":2}"#),
        // Rule 5.
        set("std", r#"{"DeadLetterQueue":"std","MaxReceiveCount":2}"#),
        set("std", r#"{"DeadLetterQueue":"f.fifo","MaxReceiveCount":2}"#),
        set("std", r#"{"DeadLetterQueue":"other","MaxReceiveCount":2}"#),
        // Value errors come before the queue lookup.
        set("missing", r#"{"DelaySeconds":901}"#),
        set("missing", r#"{"Colour":1}"#),
        r#"{"Command":"SetQueueAttributes","QueueName":"std"}"#.to_owned(),
        r#"{"Command":"SetQueueAttributes","QueueName":"std","Attributes":[]}"#.to_owned(),
    ]);
    let codes: Vec<_> = responses[3..]
        .iter()
        .map(|r| if r["ok"] == true { "ok" } else { error_code(r) })
        .collect();
    assert_eq!(
        codes,
        [
            "QueueDoesNotExist",
            "UnsupportedOperation",
            "UnsupportedOperation",
            "InvalidParameterValue",
            "InvalidParameterValue",
            "InvalidParameterValue",
            "QueueDoesNotExist",
            "InvalidParameterValue",
            "InvalidParameterValue",
            "ok",
            "InvalidParameterValue",
            "InvalidParameterValue",
            "MissingParameter",
            "InvalidParameterValue",
        ]
    );
}

#[test]
fn a_dead_letter_configuration_can_be_replaced_but_not_removed() {
    let responses = run_owned(&[
        create("a", "{}"),
        create("b", "{}"),
        create("q", r#"{"DeadLetterQueue":"a","MaxReceiveCount":2}"#),
        set("q", r#"{"VisibilityTimeout":1}"#),
        get("q"),
        set("q", r#"{"DeadLetterQueue":"b","MaxReceiveCount":5}"#),
        get("q"),
        set("q", r#"{"DeadLetterQueue":null,"MaxReceiveCount":null}"#),
        get("q"),
    ]);
    assert_eq!(responses[4]["Attributes"]["DeadLetterQueue"], "a");
    assert_eq!(responses[4]["Attributes"]["MaxReceiveCount"], 2);
    assert_eq!(responses[6]["Attributes"]["DeadLetterQueue"], "b");
    assert_eq!(responses[6]["Attributes"]["MaxReceiveCount"], 5);
    assert_eq!(responses[8]["Attributes"]["DeadLetterQueue"], "b");
}

#[test]
fn list_queues_filters_by_prefix_in_byte_order() {
    let names = [
        "b", "a", "_x", "Zed", "a-1", "a_1", "a1", "a.fifo", "10", "ab",
    ];
    let mut lines: Vec<String> = names
        .iter()
        .map(|name| {
            let fifo = name.ends_with(".fifo");
            create(name, &format!(r#"{{"Fifo":{fifo}}}"#))
        })
        .collect();
    lines.push(r#"{"Command":"ListQueues"}"#.to_owned());
    lines.push(r#"{"Command":"ListQueues","Prefix":"a"}"#.to_owned());
    lines.push(r#"{"Command":"ListQueues","Prefix":""}"#.to_owned());
    lines.push(r#"{"Command":"ListQueues","Prefix":"zz"}"#.to_owned());
    lines.push(r#"{"Command":"ListQueues","Prefix":"A"}"#.to_owned());
    let responses = run_owned(&lines);
    let n = names.len();
    assert_eq!(
        responses[n],
        json!({"ok": true, "QueueNames": ["10", "Zed", "_x", "a", "a-1", "a.fifo", "a1", "a_1", "ab", "b"]})
    );
    assert_eq!(
        responses[n + 1],
        json!({"ok": true, "QueueNames": ["a", "a-1", "a.fifo", "a1", "a_1", "ab"]})
    );
    assert_eq!(responses[n + 2], responses[n]);
    assert_eq!(responses[n + 3], json!({"ok": true, "QueueNames": []}));
    assert_eq!(responses[n + 4], json!({"ok": true, "QueueNames": []}));
}

#[test]
fn deleting_a_queue_clears_the_dead_letter_configuration_that_names_it() {
    let responses = run(&[
        &create("dlq", "{}"),
        &create("q", r#"{"DeadLetterQueue":"dlq","MaxReceiveCount":2}"#),
        r#"{"Command":"DeleteQueue","QueueName":"dlq"}"#,
        &get("q"),
        r#"{"Command":"DeleteQueue","QueueName":"dlq"}"#,
        // A new queue of the same name is unrelated to the old one.
        &create("dlq", "{}"),
        &get("q"),
    ]);
    assert_eq!(responses[2], json!({"ok": true}));
    let unset = json!({
        "VisibilityTimeout": 30,
        "DelaySeconds": 0,
        "RetentionSeconds": 345600,
        "Fifo": false,
        "ContentBasedDeduplication": false
    });
    assert_eq!(responses[3]["Attributes"], unset);
    assert_eq!(error_code(&responses[4]), "QueueDoesNotExist");
    assert_eq!(responses[6]["Attributes"], unset);
}

#[test]
fn queue_commands_report_a_missing_queue() {
    let responses = run(&[
        r#"{"Command":"GetQueueAttributes","QueueName":"q"}"#,
        r#"{"Command":"DeleteQueue","QueueName":"q"}"#,
        r#"{"Command":"PurgeQueue","QueueName":"q"}"#,
        r#"{"Command":"SendMessage","QueueName":"q","Body":"b"}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"q"}"#,
        r#"{"Command":"ChangeMessageVisibility","QueueName":"q","ReceiptHandle":"r-1","VisibilityTimeout":1}"#,
    ]);
    for response in &responses {
        assert_eq!(error_code(response), "QueueDoesNotExist");
    }
}

#[test]
fn queue_names_are_case_sensitive() {
    let responses = run(&[&create("Jobs", "{}"), &get("jobs"), &get("Jobs")]);
    assert_eq!(error_code(&responses[1]), "QueueDoesNotExist");
    assert_eq!(responses[2]["ok"], true);
}

#[test]
fn purge_empties_the_queue_but_keeps_sequence_numbers_and_deduplication() {
    let send = |body: &str, group: &str| {
        format!(
            r#"{{"Command":"SendMessage","QueueName":"f.fifo","Body":"{body}","GroupId":"{group}","DeduplicationId":"{body}"}}"#
        )
    };
    let responses = run_owned(&[
        create("f.fifo", r#"{"Fifo":true}"#),
        send("one", "g"),
        send("two", "g"),
        r#"{"Command":"PurgeQueue","QueueName":"f.fifo"}"#.to_owned(),
        get("f.fifo"),
        send("two", "g"),
        send("three", "g"),
        r#"{"Command":"ReceiveMessage","QueueName":"f.fifo","MaxMessages":10}"#.to_owned(),
    ]);
    assert_eq!(responses[3], json!({"ok": true}));
    assert_eq!(
        responses[4]["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
    // "two" is still a duplicate: the original id and sequence number come back.
    assert_eq!(
        responses[5],
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2})
    );
    assert_eq!(
        responses[6],
        json!({"ok": true, "MessageId": "m-3", "SequenceNumber": 3})
    );
    let messages = responses[7]["Messages"].as_array().unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["Body"], "three");
}
