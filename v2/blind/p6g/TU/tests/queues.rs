mod common;

use common::*;
use serde_json::{Value, json};

fn attributes(responses: &Value) -> &Value {
    &responses["Attributes"]
}

#[test]
fn a_new_queue_has_the_default_attributes() {
    let responses = run(&[create("jobs"), counts("jobs")]);
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
                "ContentBasedDeduplication": false
            },
            "Counts": {"Visible": 0, "InFlight": 0, "Delayed": 0}
        })
    );
}

#[test]
fn queue_names_follow_the_syntax_and_fifo_suffix_rule() {
    let long = "a".repeat(80);
    let responses = run(&[
        create(&long),
        create(&"a".repeat(81)),
        create("a b"),
        create("jobs.fifo"),
        create_with("jobs.fifo", json!({"Fifo": false})),
        create_with("plain", json!({"Fifo": true})),
        create_with("ok.fifo", json!({"Fifo": true})),
        create(&format!("{}.fifo", "a".repeat(76))),
    ]);
    let results: Vec<_> = responses.iter().map(outcome).collect();
    assert_eq!(
        results,
        [
            "ok",
            "QueueNameInvalid",
            "QueueNameInvalid",
            "QueueNameInvalid",
            "QueueNameInvalid",
            "QueueNameInvalid",
            "ok",
            "QueueNameInvalid"
        ]
    );
}

#[test]
fn create_is_idempotent_for_equal_attributes_and_rejects_different_ones() {
    let responses = run(&[
        create_with("q", json!({"VisibilityTimeout": 10})),
        create_with("q", json!({"VisibilityTimeout": 10, "DelaySeconds": 0})),
        create_with("q", json!({"VisibilityTimeout": 11})),
        create("q"),
        create_with("q", json!({"VisibilityTimeout": null})),
    ]);
    let codes: Vec<_> = responses.iter().map(outcome).collect();
    assert_eq!(
        codes,
        [
            "ok",
            "ok",
            "QueueAlreadyExists",
            "QueueAlreadyExists",
            "QueueAlreadyExists"
        ]
    );
}

#[test]
fn create_rules_apply_in_the_documented_order() {
    let responses = run(&[
        create("dlq"),
        create("std"),
        create_with("fifo.fifo", json!({"Fifo": true})),
        // Rule 2 before rule 3.
        create_with(
            "a",
            json!({"ContentBasedDeduplication": true, "MaxReceiveCount": 3}),
        ),
        // Rule 3.
        create_with("a", json!({"MaxReceiveCount": 3})),
        create_with("a", json!({"DeadLetterQueue": "dlq"})),
        // Rule 4 before rule 5: "std" exists with different attributes.
        create_with(
            "std",
            json!({"DeadLetterQueue": "missing", "MaxReceiveCount": 3}),
        ),
        // Rule 5.
        create_with(
            "a",
            json!({"DeadLetterQueue": "missing", "MaxReceiveCount": 3}),
        ),
        // Rule 6: dead-letter queue of the other type.
        create_with(
            "a",
            json!({"DeadLetterQueue": "fifo.fifo", "MaxReceiveCount": 3}),
        ),
        create_with(
            "b.fifo",
            json!({"Fifo": true, "DeadLetterQueue": "dlq", "MaxReceiveCount": 3}),
        ),
        // Valid, and equal when repeated.
        create_with("c", json!({"DeadLetterQueue": "dlq", "MaxReceiveCount": 3})),
        create_with("c", json!({"DeadLetterQueue": "dlq", "MaxReceiveCount": 3})),
        create_with("c", json!({"DeadLetterQueue": "dlq", "MaxReceiveCount": 4})),
        create_with("c", json!({"ContentBasedDeduplication": true})),
    ]);
    let codes: Vec<_> = responses.iter().map(outcome).collect();
    assert_eq!(
        codes,
        [
            "ok",
            "ok",
            "ok",
            "InvalidParameterValue",
            "InvalidParameterValue",
            "InvalidParameterValue",
            "QueueAlreadyExists",
            "QueueDoesNotExist",
            "InvalidParameterValue",
            "InvalidParameterValue",
            "ok",
            "ok",
            "QueueAlreadyExists",
            "InvalidParameterValue"
        ]
    );
}

#[test]
fn dead_letter_configuration_is_reported_only_when_set() {
    let responses = run(&[
        create("dlq"),
        create_with("q", json!({"DeadLetterQueue": "dlq", "MaxReceiveCount": 2})),
        counts("q"),
    ]);
    assert_eq!(attributes(&responses[2])["DeadLetterQueue"], json!("dlq"));
    assert_eq!(attributes(&responses[2])["MaxReceiveCount"], json!(2));
}

#[test]
fn queue_limit_is_1000() {
    let mut commands: Vec<Value> = (0..1000).map(|i| create(&format!("q{i}"))).collect();
    commands.push(create("one-too-many"));
    commands.push(create("q0"));
    commands.push(create_with("q1", json!({"DelaySeconds": 1})));
    let responses = run(&commands);
    assert!(responses[..1000].iter().all(|r| *r == ok()));
    assert_eq!(code(&responses[1000]), "LimitExceeded");
    assert_eq!(responses[1001], ok());
    assert_eq!(code(&responses[1002]), "QueueAlreadyExists");
}

#[test]
fn set_attributes_changes_only_the_attributes_given() {
    let responses = run(&[
        create_with("q", json!({"VisibilityTimeout": 5, "DelaySeconds": 7})),
        cmd(
            "SetQueueAttributes",
            json!({"QueueName": "q", "Attributes": {"DelaySeconds": 8, "RetentionSeconds": null}}),
        ),
        cmd(
            "SetQueueAttributes",
            json!({"QueueName": "q", "Attributes": {}}),
        ),
        counts("q"),
    ]);
    assert_eq!(responses[1], ok());
    assert_eq!(responses[2], ok());
    assert_eq!(attributes(&responses[3])["VisibilityTimeout"], json!(5));
    assert_eq!(attributes(&responses[3])["DelaySeconds"], json!(8));
    assert_eq!(attributes(&responses[3])["RetentionSeconds"], json!(345600));
}

#[test]
fn set_attributes_rules_apply_in_the_documented_order() {
    let set = |queue: &str, attrs: Value| {
        cmd(
            "SetQueueAttributes",
            json!({"QueueName": queue, "Attributes": attrs}),
        )
    };
    let responses = run(&[
        create("a"),
        create("b"),
        create_with("f.fifo", json!({"Fifo": true})),
        set("a", json!({"Fifo": false})),
        set(
            "a",
            json!({"Fifo": true, "ContentBasedDeduplication": true}),
        ),
        set("a", json!({"ContentBasedDeduplication": true})),
        set("a", json!({"ContentBasedDeduplication": false})),
        set("a", json!({"MaxReceiveCount": 2})),
        set(
            "a",
            json!({"DeadLetterQueue": "missing", "MaxReceiveCount": 2}),
        ),
        set("a", json!({"DeadLetterQueue": "a", "MaxReceiveCount": 2})),
        set(
            "a",
            json!({"DeadLetterQueue": "f.fifo", "MaxReceiveCount": 2}),
        ),
        set("a", json!({"DeadLetterQueue": "b", "MaxReceiveCount": 2})),
        set("a", json!({"DeadLetterQueue": "b", "MaxReceiveCount": 9})),
        set("missing", json!({})),
        cmd("SetQueueAttributes", json!({"QueueName": "a"})),
        set("f.fifo", json!({"ContentBasedDeduplication": true})),
    ]);
    let codes: Vec<_> = responses[3..].iter().map(outcome).collect();
    assert_eq!(
        codes,
        [
            "UnsupportedOperation",
            "UnsupportedOperation",
            "InvalidParameterValue",
            "ok",
            "InvalidParameterValue",
            "QueueDoesNotExist",
            "InvalidParameterValue",
            "InvalidParameterValue",
            "ok",
            "ok",
            "QueueDoesNotExist",
            "MissingParameter",
            "ok"
        ]
    );
}

#[test]
fn list_queues_sorts_by_bytes_and_filters_by_prefix() {
    let names = ["b", "B", "_x", "a.fifo", "a-b", "a1", "a"];
    let mut commands: Vec<Value> = names
        .iter()
        .map(|n| {
            if n.ends_with(".fifo") {
                create_with(n, json!({"Fifo": true}))
            } else {
                create(n)
            }
        })
        .collect();
    commands.push(cmd("ListQueues", json!({})));
    commands.push(cmd("ListQueues", json!({"Prefix": "a"})));
    commands.push(cmd("ListQueues", json!({"Prefix": "a."})));
    commands.push(cmd("ListQueues", json!({"Prefix": "zz"})));
    commands.push(cmd("ListQueues", json!({"Prefix": 4})));
    let responses = run(&commands);
    let n = names.len();
    assert_eq!(
        responses[n],
        json!({"ok": true, "QueueNames": ["B", "_x", "a", "a-b", "a.fifo", "a1", "b"]})
    );
    assert_eq!(
        responses[n + 1]["QueueNames"],
        json!(["a", "a-b", "a.fifo", "a1"])
    );
    assert_eq!(responses[n + 2]["QueueNames"], json!(["a.fifo"]));
    assert_eq!(responses[n + 3]["QueueNames"], json!([]));
    assert_eq!(code(&responses[n + 4]), "InvalidParameterValue");
}

#[test]
fn deleting_a_queue_removes_the_dead_letter_configuration_pointing_at_it() {
    let responses = run(&[
        create("dlq"),
        create_with("q", json!({"DeadLetterQueue": "dlq", "MaxReceiveCount": 2})),
        cmd("DeleteQueue", json!({"QueueName": "dlq"})),
        counts("q"),
        create("dlq"),
        cmd("DeleteQueue", json!({"QueueName": "dlq"})),
        cmd("DeleteQueue", json!({"QueueName": "dlq"})),
        // With the configuration gone, an unset one is what a plain request means.
        create("q"),
    ]);
    assert_eq!(responses[2], ok());
    assert_eq!(
        attributes(&responses[3]).as_object().unwrap().keys().len(),
        5,
        "{}",
        responses[3]
    );
    assert_eq!(code(&responses[6]), "QueueDoesNotExist");
    assert_eq!(responses[7], ok());
}

#[test]
fn a_recreated_queue_shares_nothing_with_the_deleted_one() {
    let responses = run(&[
        create("q"),
        send("q", "old"),
        cmd("DeleteQueue", json!({"QueueName": "q"})),
        create("q"),
        counts("q"),
        send("q", "new"),
    ]);
    assert_eq!(
        responses[4]["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(responses[5], json!({"ok": true, "MessageId": "m-2"}));
}

#[test]
fn purge_removes_every_message_whatever_its_state() {
    let responses = run(&[
        create("q"),
        send("q", "a"),
        send("q", "b"),
        cmd(
            "SendMessage",
            json!({"QueueName": "q", "Body": "c", "DelaySeconds": 10}),
        ),
        receive("q"),
        cmd("PurgeQueue", json!({"QueueName": "q"})),
        counts("q"),
        cmd("PurgeQueue", json!({"QueueName": "missing"})),
    ]);
    assert_eq!(responses[5], ok());
    assert_eq!(
        responses[6]["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(code(&responses[7]), "QueueDoesNotExist");
}
