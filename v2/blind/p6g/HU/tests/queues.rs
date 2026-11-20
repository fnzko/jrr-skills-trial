mod common;

use common::Session;
use serde_json::json;

fn create(s: &mut Session, name: &str) {
    s.ok(&format!(
        r#"{{"Command": "CreateQueue", "QueueName": "{name}"}}"#
    ));
}

#[test]
fn create_succeeds_with_no_result_members() {
    let mut s = Session::new();
    assert_eq!(
        s.send(r#"{"Command": "CreateQueue", "QueueName": "jobs"}"#),
        json!({"ok": true})
    );
}

#[test]
fn members_that_are_null_or_undefined_are_ignored() {
    let mut s = Session::new();
    s.ok(r#"{"Command": "CreateQueue", "QueueName": "a", "Attributes": null, "Extra": 1}"#);
    s.ok(r#"{"Command": "CreateQueue", "QueueName": "b", "Attributes": {"DelaySeconds": null, "Fifo": null}}"#);
}

#[test]
fn queue_name_rules() {
    let mut s = Session::new();
    let max_standard = "a".repeat(80);
    let max_fifo = format!("{}.fifo", "b".repeat(75));
    for name in ["x", "A-z_9", max_standard.as_str()] {
        create(&mut s, name);
    }
    s.ok(&format!(
        r#"{{"Command": "CreateQueue", "QueueName": "{max_fifo}", "Attributes": {{"Fifo": true}}}}"#
    ));

    let too_long = "a".repeat(81);
    let too_long_fifo = format!("{}.fifo", "b".repeat(76));
    for name in [
        "",
        "a b",
        "a.b",
        ".fifo",
        "é",
        "a.fifo.fifo",
        &too_long,
        &too_long_fifo,
    ] {
        let line = json!({"Command": "CreateQueue", "QueueName": name}).to_string();
        assert_eq!(s.err(&line), "QueueNameInvalid", "name {name:?}");
    }
}

#[test]
fn fifo_flag_and_suffix_must_agree() {
    let mut s = Session::new();
    assert_eq!(
        s.err(r#"{"Command": "CreateQueue", "QueueName": "a.fifo"}"#),
        "QueueNameInvalid"
    );
    assert_eq!(
        s.err(r#"{"Command": "CreateQueue", "QueueName": "a", "Attributes": {"Fifo": true}}"#),
        "QueueNameInvalid"
    );
    s.ok(r#"{"Command": "CreateQueue", "QueueName": "a.fifo", "Attributes": {"Fifo": true}}"#);
    s.ok(r#"{"Command": "CreateQueue", "QueueName": "b", "Attributes": {"Fifo": false}}"#);
}

#[test]
fn create_checks_follow_the_documented_order() {
    let mut s = Session::new();
    // Missing beats invalid values.
    assert_eq!(
        s.err(r#"{"Command": "CreateQueue", "Attributes": 5}"#),
        "MissingParameter"
    );
    // The name is checked before every other parameter.
    assert_eq!(
        s.err(r#"{"Command": "CreateQueue", "QueueName": "bad name", "Attributes": 5}"#),
        "QueueNameInvalid"
    );
    assert_eq!(
        s.err(r#"{"Command": "CreateQueue", "QueueName": 5}"#),
        "InvalidParameterValue"
    );
    // Value errors come before the name/Fifo rule.
    assert_eq!(
        s.err(r#"{"Command": "CreateQueue", "QueueName": "a.fifo", "Attributes": {"DelaySeconds": 901}}"#),
        "InvalidParameterValue"
    );
    // Rule 1 comes before rule 2, rule 2 before rule 3.
    assert_eq!(
        s.err(r#"{"Command": "CreateQueue", "QueueName": "a", "Attributes": {"Fifo": true, "ContentBasedDeduplication": true}}"#),
        "QueueNameInvalid"
    );
    assert_eq!(
        s.err(r#"{"Command": "CreateQueue", "QueueName": "a", "Attributes": {"ContentBasedDeduplication": true, "MaxReceiveCount": 3}}"#),
        "InvalidParameterValue"
    );
    // Rule 3 comes before the dead-letter queue lookup.
    assert_eq!(
        s.err(r#"{"Command": "CreateQueue", "QueueName": "a", "Attributes": {"DeadLetterQueue": "nope"}}"#),
        "InvalidParameterValue"
    );
    assert_eq!(
        s.err(r#"{"Command": "CreateQueue", "QueueName": "a", "Attributes": {"DeadLetterQueue": "nope", "MaxReceiveCount": 2}}"#),
        "QueueDoesNotExist"
    );
}

#[test]
fn attribute_values_are_validated() {
    let mut s = Session::new();
    let bad = [
        r#""Attributes": []"#,
        r#""Attributes": "x""#,
        r#""Attributes": {"Bogus": 1}"#,
        r#""Attributes": {"VisibilityTimeout": -1}"#,
        r#""Attributes": {"VisibilityTimeout": 43201}"#,
        r#""Attributes": {"VisibilityTimeout": 30.0}"#,
        r#""Attributes": {"VisibilityTimeout": 3e1}"#,
        r#""Attributes": {"VisibilityTimeout": "30"}"#,
        r#""Attributes": {"DelaySeconds": 901}"#,
        r#""Attributes": {"RetentionSeconds": 59}"#,
        r#""Attributes": {"RetentionSeconds": 1209601}"#,
        r#""Attributes": {"RetentionSeconds": 99999999999999999999}"#,
        r#""Attributes": {"Fifo": "true"}"#,
        r#""Attributes": {"Fifo": 1}"#,
        r#""Attributes": {"DeadLetterQueue": 5, "MaxReceiveCount": 1}"#,
        r#""Attributes": {"DeadLetterQueue": "d", "MaxReceiveCount": 0}"#,
        r#""Attributes": {"DeadLetterQueue": "d", "MaxReceiveCount": 1001}"#,
    ];
    for attributes in bad {
        let line = format!(r#"{{"Command": "CreateQueue", "QueueName": "q", {attributes}}}"#);
        assert_eq!(s.err(&line), "InvalidParameterValue", "{attributes}");
    }
    s.ok(r#"{"Command": "CreateQueue", "QueueName": "edges", "Attributes": {"VisibilityTimeout": 43200, "DelaySeconds": 900, "RetentionSeconds": 60}}"#);
}

#[test]
fn create_is_idempotent_only_for_equal_attributes() {
    let mut s = Session::new();
    s.ok(r#"{"Command": "CreateQueue", "QueueName": "q", "Attributes": {"VisibilityTimeout": 5}}"#);
    s.ok(r#"{"Command": "CreateQueue", "QueueName": "q", "Attributes": {"VisibilityTimeout": 5, "DelaySeconds": 0}}"#);
    assert_eq!(
        s.err(r#"{"Command": "CreateQueue", "QueueName": "q"}"#),
        "QueueAlreadyExists"
    );
    assert_eq!(
        s.err(r#"{"Command": "CreateQueue", "QueueName": "q", "Attributes": {"VisibilityTimeout": 6}}"#),
        "QueueAlreadyExists"
    );
}

#[test]
fn create_with_dead_letter_queue_checks_the_target() {
    let mut s = Session::new();
    create(&mut s, "dead");
    s.ok(r#"{"Command": "CreateQueue", "QueueName": "q", "Attributes": {"DeadLetterQueue": "dead", "MaxReceiveCount": 3}}"#);
    // Same attributes again: equal.
    s.ok(r#"{"Command": "CreateQueue", "QueueName": "q", "Attributes": {"DeadLetterQueue": "dead", "MaxReceiveCount": 3}}"#);
    assert_eq!(
        s.err(r#"{"Command": "CreateQueue", "QueueName": "q", "Attributes": {"DeadLetterQueue": "dead", "MaxReceiveCount": 4}}"#),
        "QueueAlreadyExists"
    );
    // A FIFO queue cannot dead-letter into a standard queue.
    assert_eq!(
        s.err(r#"{"Command": "CreateQueue", "QueueName": "f.fifo", "Attributes": {"Fifo": true, "DeadLetterQueue": "dead", "MaxReceiveCount": 3}}"#),
        "InvalidParameterValue"
    );
    // A queue cannot name itself before it exists.
    assert_eq!(
        s.err(r#"{"Command": "CreateQueue", "QueueName": "z", "Attributes": {"DeadLetterQueue": "z", "MaxReceiveCount": 3}}"#),
        "QueueDoesNotExist"
    );
}

#[test]
fn queue_limit_is_1000() {
    let mut s = Session::new();
    for i in 0..1000 {
        create(&mut s, &format!("q{i}"));
    }
    assert_eq!(
        s.err(r#"{"Command": "CreateQueue", "QueueName": "extra"}"#),
        "LimitExceeded"
    );
    // Re-creating an existing queue is not a new queue.
    create(&mut s, "q0");
    s.ok(r#"{"Command": "DeleteQueue", "QueueName": "q0"}"#);
    create(&mut s, "extra");
}

#[test]
fn list_queues_sorts_by_byte_value_and_filters_by_prefix() {
    let mut s = Session::new();
    for name in ["b", "a_", "a-", "A", "a1", "a", "a.fifo"] {
        if name.ends_with(".fifo") {
            s.ok(&format!(
                r#"{{"Command": "CreateQueue", "QueueName": "{name}", "Attributes": {{"Fifo": true}}}}"#
            ));
        } else {
            create(&mut s, name);
        }
    }
    assert_eq!(
        s.ok(r#"{"Command": "ListQueues"}"#),
        json!({"QueueNames": ["A", "a", "a-", "a.fifo", "a1", "a_", "b"]})
    );
    assert_eq!(
        s.ok(r#"{"Command": "ListQueues", "Prefix": "a"}"#),
        json!({"QueueNames": ["a", "a-", "a.fifo", "a1", "a_"]})
    );
    assert_eq!(
        s.ok(r#"{"Command": "ListQueues", "Prefix": ""}"#)["QueueNames"]
            .as_array()
            .unwrap()
            .len(),
        7
    );
    assert_eq!(
        s.ok(r#"{"Command": "ListQueues", "Prefix": "zz"}"#),
        json!({"QueueNames": []})
    );
    assert_eq!(
        s.err(r#"{"Command": "ListQueues", "Prefix": 1}"#),
        "InvalidParameterValue"
    );
}

#[test]
fn delete_removes_the_queue_and_clears_dead_letter_configs_that_name_it() {
    let mut s = Session::new();
    create(&mut s, "dead");
    s.ok(r#"{"Command": "CreateQueue", "QueueName": "q", "Attributes": {"DeadLetterQueue": "dead", "MaxReceiveCount": 3}}"#);
    s.ok(r#"{"Command": "DeleteQueue", "QueueName": "dead"}"#);
    assert_eq!(
        s.err(r#"{"Command": "DeleteQueue", "QueueName": "dead"}"#),
        "QueueDoesNotExist"
    );
    // Both attributes are unset again, so a plain create now equals q.
    s.ok(r#"{"Command": "CreateQueue", "QueueName": "q"}"#);
    assert_eq!(s.err(r#"{"Command": "DeleteQueue"}"#), "MissingParameter");
    assert_eq!(
        s.err(r#"{"Command": "DeleteQueue", "QueueName": "Dead"}"#),
        "QueueDoesNotExist"
    );
}

#[test]
fn set_queue_attributes_rules() {
    let mut s = Session::new();
    create(&mut s, "q");
    create(&mut s, "dead");
    s.ok(r#"{"Command": "CreateQueue", "QueueName": "f.fifo", "Attributes": {"Fifo": true}}"#);

    assert_eq!(
        s.err(r#"{"Command": "SetQueueAttributes", "QueueName": "q"}"#),
        "MissingParameter"
    );
    assert_eq!(
        s.err(r#"{"Command": "SetQueueAttributes", "QueueName": "nope", "Attributes": {}}"#),
        "QueueDoesNotExist"
    );
    assert_eq!(
        s.err(r#"{"Command": "SetQueueAttributes", "QueueName": "nope", "Attributes": {"DelaySeconds": 1000}}"#),
        "InvalidParameterValue"
    );
    assert_eq!(
        s.err(
            r#"{"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"Fifo": false}}"#
        ),
        "UnsupportedOperation"
    );
    assert_eq!(
        s.err(r#"{"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"Fifo": 1}}"#),
        "InvalidParameterValue"
    );
    assert_eq!(
        s.err(r#"{"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"ContentBasedDeduplication": true}}"#),
        "InvalidParameterValue"
    );
    assert_eq!(
        s.err(r#"{"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"MaxReceiveCount": 2}}"#),
        "InvalidParameterValue"
    );
    assert_eq!(
        s.err(r#"{"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"DeadLetterQueue": "nope", "MaxReceiveCount": 2}}"#),
        "QueueDoesNotExist"
    );
    assert_eq!(
        s.err(r#"{"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"DeadLetterQueue": "q", "MaxReceiveCount": 2}}"#),
        "InvalidParameterValue"
    );
    assert_eq!(
        s.err(r#"{"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"DeadLetterQueue": "f.fifo", "MaxReceiveCount": 2}}"#),
        "InvalidParameterValue"
    );
    s.ok(r#"{"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {}}"#);
    s.ok(r#"{"Command": "SetQueueAttributes", "QueueName": "f.fifo", "Attributes": {"ContentBasedDeduplication": true}}"#);
}

#[test]
fn set_queue_attributes_changes_only_the_given_attributes() {
    let mut s = Session::new();
    create(&mut s, "dead");
    s.ok(r#"{"Command": "CreateQueue", "QueueName": "q", "Attributes": {"DelaySeconds": 7}}"#);
    s.ok(r#"{"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"VisibilityTimeout": 9, "DeadLetterQueue": "dead", "MaxReceiveCount": 2}}"#);
    // Visible through the idempotence rule of CreateQueue.
    s.ok(r#"{"Command": "CreateQueue", "QueueName": "q", "Attributes": {"VisibilityTimeout": 9, "DelaySeconds": 7, "DeadLetterQueue": "dead", "MaxReceiveCount": 2}}"#);
    // The dead-letter configuration can be replaced, and null removes nothing.
    create(&mut s, "dead2");
    s.ok(r#"{"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"DeadLetterQueue": "dead2", "MaxReceiveCount": 5, "DelaySeconds": null}}"#);
    s.ok(r#"{"Command": "CreateQueue", "QueueName": "q", "Attributes": {"VisibilityTimeout": 9, "DelaySeconds": 7, "DeadLetterQueue": "dead2", "MaxReceiveCount": 5}}"#);
}

#[test]
fn tick_advances_the_clock() {
    let mut s = Session::new();
    assert_eq!(
        s.ok(r#"{"Command": "Tick", "Seconds": 0}"#),
        json!({"Now": 0})
    );
    assert_eq!(
        s.ok(r#"{"Command": "Tick", "Seconds": 5}"#),
        json!({"Now": 5})
    );
    assert_eq!(
        s.ok(r#"{"Command": "Tick", "Seconds": 1209600}"#),
        json!({"Now": 1209605})
    );
    assert_eq!(s.err(r#"{"Command": "Tick"}"#), "MissingParameter");
    for seconds in ["-1", "1209601", "1.5", "1e2", "\"1\"", "true"] {
        let line = format!(r#"{{"Command": "Tick", "Seconds": {seconds}}}"#);
        assert_eq!(s.err(&line), "InvalidParameterValue", "{seconds}");
    }
    assert_eq!(
        s.ok(r#"{"Command": "Tick", "Seconds": 0}"#),
        json!({"Now": 1209605})
    );
}
