use crate::common::*;
use serde_json::json;

#[test]
fn get_queue_attributes_reports_defaults_and_counts() {
    let responses = run(&[create("jobs"), attributes("jobs")]);
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
fn dead_letter_attributes_appear_only_when_set() {
    let responses = run(&[
        create("dead"),
        create_with(
            "jobs",
            json!({"DeadLetterQueue": "dead", "MaxReceiveCount": 3}),
        ),
        attributes("jobs"),
    ]);
    assert_eq!(
        responses[2]["Attributes"],
        json!({
            "VisibilityTimeout": 30,
            "DelaySeconds": 0,
            "RetentionSeconds": 345600,
            "Fifo": false,
            "ContentBasedDeduplication": false,
            "DeadLetterQueue": "dead",
            "MaxReceiveCount": 3,
        })
    );
}

#[test]
fn create_queue_errors_follow_rule_order() {
    let cases = [
        (create("a b"), "QueueNameInvalid"),
        (create("q.fifo"), "QueueNameInvalid"),
        (create_with("q", json!({"Fifo": true})), "QueueNameInvalid"),
        (
            create_with("q", json!({"ContentBasedDeduplication": true})),
            "InvalidParameterValue",
        ),
        (
            create_with("q", json!({"DeadLetterQueue": "x"})),
            "InvalidParameterValue",
        ),
        (
            create_with("q", json!({"DeadLetterQueue": "x", "MaxReceiveCount": 2})),
            "QueueDoesNotExist",
        ),
        (
            create_with(
                "q.fifo",
                json!({"Fifo": true, "DeadLetterQueue": "std", "MaxReceiveCount": 2}),
            ),
            "InvalidParameterValue",
        ),
    ];
    for (request, expected) in cases {
        let responses = run(&[create("std"), request.clone()]);
        assert_eq!(code(&responses[1]), expected, "{request}");
    }
}

#[test]
fn recreating_with_equal_attributes_is_a_no_op_and_otherwise_conflicts() {
    let responses = run(&[
        create_with("q", json!({"VisibilityTimeout": 5})),
        send("q", "kept"),
        create_with("q", json!({"VisibilityTimeout": 5, "DelaySeconds": 0})),
        create("q"),
        attributes("q"),
    ]);
    assert_eq!(responses[2], ok());
    assert_eq!(code(&responses[3]), "QueueAlreadyExists");
    assert_eq!(
        counts(&responses[4]),
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn existing_queue_is_decided_before_the_dead_letter_queue_lookup() {
    // Rule 4 (exists) is decided before rule 5 (dead-letter queue exists).
    let responses = run(&[
        create("q"),
        create_with(
            "q",
            json!({"DeadLetterQueue": "nope", "MaxReceiveCount": 1}),
        ),
    ]);
    assert_eq!(code(&responses[1]), "QueueAlreadyExists");
}

#[test]
fn creating_the_1001st_queue_fails() {
    let mut lines: Vec<_> = (0..1000).map(|n| create(&format!("q{n}"))).collect();
    lines.push(create("one-too-many"));
    lines.push(create("q0"));
    let responses = run(&lines);
    assert!(responses[..1000].iter().all(|r| *r == ok()));
    assert_eq!(code(&responses[1000]), "LimitExceeded");
    assert_eq!(responses[1001], ok());
}

#[test]
fn set_queue_attributes_applies_only_what_is_given() {
    let responses = run(&[
        create_with("q", json!({"DelaySeconds": 4})),
        set_attributes(
            "q",
            json!({"VisibilityTimeout": 9, "RetentionSeconds": 100}),
        ),
        attributes("q"),
    ]);
    assert_eq!(responses[1], ok());
    assert_eq!(
        responses[2]["Attributes"],
        json!({
            "VisibilityTimeout": 9,
            "DelaySeconds": 4,
            "RetentionSeconds": 100,
            "Fifo": false,
            "ContentBasedDeduplication": false,
        })
    );
}

#[test]
fn set_queue_attributes_errors() {
    let cases = [
        (
            set_attributes("missing", json!({"Fifo": true})),
            "QueueDoesNotExist",
        ),
        (
            set_attributes("q", json!({"Fifo": false})),
            "UnsupportedOperation",
        ),
        (
            set_attributes("q", json!({"ContentBasedDeduplication": true})),
            "InvalidParameterValue",
        ),
        (
            set_attributes("q", json!({"MaxReceiveCount": 2})),
            "InvalidParameterValue",
        ),
        (
            set_attributes(
                "q",
                json!({"DeadLetterQueue": "nope", "MaxReceiveCount": 2}),
            ),
            "QueueDoesNotExist",
        ),
        (
            set_attributes("q", json!({"DeadLetterQueue": "q", "MaxReceiveCount": 2})),
            "InvalidParameterValue",
        ),
        (
            set_attributes(
                "q",
                json!({"DeadLetterQueue": "f.fifo", "MaxReceiveCount": 2}),
            ),
            "InvalidParameterValue",
        ),
        (
            set_attributes("q", json!({"Nope": 1})),
            "InvalidParameterValue",
        ),
        (set_attributes("q", json!(null)), "MissingParameter"),
    ];
    for (request, expected) in cases {
        let responses = run(&[
            create("q"),
            create_with("f.fifo", json!({"Fifo": true})),
            request.clone(),
        ]);
        assert_eq!(code(&responses[2]), expected, "{request}");
    }
}

#[test]
fn failed_set_queue_attributes_changes_nothing() {
    let responses = run(&[
        create("q"),
        set_attributes(
            "q",
            json!({"VisibilityTimeout": 1, "DeadLetterQueue": "nope"}),
        ),
        attributes("q"),
    ]);
    assert_eq!(code(&responses[1]), "InvalidParameterValue");
    assert_eq!(responses[2]["Attributes"]["VisibilityTimeout"], json!(30));
}

#[test]
fn list_queues_filters_by_prefix_in_byte_order() {
    let names = ["b", "a_", "a-", "a.fifo", "a1", "aB", "ab"];
    let mut lines: Vec<_> = names
        .iter()
        .map(|name| {
            if name.ends_with(".fifo") {
                create_with(name, json!({"Fifo": true}))
            } else {
                create(name)
            }
        })
        .collect();
    lines.push(json!({"Command": "ListQueues"}));
    lines.push(json!({"Command": "ListQueues", "Prefix": "a"}));
    lines.push(json!({"Command": "ListQueues", "Prefix": "a."}));
    lines.push(json!({"Command": "ListQueues", "Prefix": "zzz"}));
    let responses = run(&lines);
    let listed: Vec<_> = responses[7..].to_vec();
    assert_eq!(
        listed,
        vec![
            json!({"ok": true, "QueueNames": ["a-", "a.fifo", "a1", "aB", "a_", "ab", "b"]}),
            json!({"ok": true, "QueueNames": ["a-", "a.fifo", "a1", "aB", "a_", "ab"]}),
            json!({"ok": true, "QueueNames": ["a.fifo"]}),
            json!({"ok": true, "QueueNames": []}),
        ]
    );
}

#[test]
fn delete_queue_unsets_dead_letter_configuration_of_others() {
    let responses = run(&[
        create("dead"),
        create_with(
            "jobs",
            json!({"DeadLetterQueue": "dead", "MaxReceiveCount": 2}),
        ),
        json!({"Command": "DeleteQueue", "QueueName": "dead"}),
        attributes("jobs"),
        json!({"Command": "DeleteQueue", "QueueName": "dead"}),
    ]);
    assert_eq!(responses[2], ok());
    assert_eq!(
        responses[3]["Attributes"],
        json!({
            "VisibilityTimeout": 30,
            "DelaySeconds": 0,
            "RetentionSeconds": 345600,
            "Fifo": false,
            "ContentBasedDeduplication": false,
        })
    );
    assert_eq!(code(&responses[4]), "QueueDoesNotExist");
}

#[test]
fn recreated_queue_shares_nothing_with_the_deleted_one() {
    let responses = run(&[
        create("q"),
        send("q", "old"),
        receive("q"),
        json!({"Command": "DeleteQueue", "QueueName": "q"}),
        create("q"),
        attributes("q"),
        delete("q", "r-1"),
    ]);
    assert_eq!(
        counts(&responses[5]),
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(code(&responses[6]), "ReceiptHandleExpired");
}

#[test]
fn purge_removes_every_state_but_keeps_sequence_numbers() {
    let responses = run(&[
        create_with("q.fifo", json!({"Fifo": true})),
        send_fifo("q.fifo", "a", "g", "d1"),
        send_fifo("q.fifo", "b", "h", "d2"),
        receive("q.fifo"),
        json!({"Command": "PurgeQueue", "QueueName": "q.fifo"}),
        attributes("q.fifo"),
        send_fifo("q.fifo", "c", "g", "d3"),
        send_fifo("q.fifo", "a again", "g", "d1"),
        delete("q.fifo", "r-1"),
    ]);
    assert_eq!(responses[4], ok());
    assert_eq!(
        counts(&responses[5]),
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(
        responses[6],
        json!({"ok": true, "MessageId": "m-3", "SequenceNumber": 3})
    );
    // d1 is still inside its deduplication window after the purge.
    assert_eq!(
        responses[7],
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
    assert_eq!(code(&responses[8]), "ReceiptHandleExpired");
}

#[test]
fn tick_returns_the_new_clock() {
    let responses = run(&[tick(5), tick(0), tick(1_209_600), tick(1_209_601)]);
    assert_eq!(responses[0], json!({"ok": true, "Now": 5}));
    assert_eq!(responses[1], json!({"ok": true, "Now": 5}));
    assert_eq!(responses[2], json!({"ok": true, "Now": 1_209_605}));
    assert_eq!(code(&responses[3]), "InvalidParameterValue");
}

#[test]
fn lookup_precedes_command_rules_for_every_queue_command() {
    let missing = "ghost";
    let cases = [
        attributes(missing),
        send(missing, "x"),
        receive(missing),
        delete(missing, "r-77"),
        change_visibility(missing, "r-77", 1),
        json!({"Command": "PurgeQueue", "QueueName": missing}),
        json!({"Command": "DeleteQueue", "QueueName": missing}),
    ];
    for request in cases {
        let responses = run(std::slice::from_ref(&request));
        assert_eq!(code(&responses[0]), "QueueDoesNotExist", "{request}");
    }
}
