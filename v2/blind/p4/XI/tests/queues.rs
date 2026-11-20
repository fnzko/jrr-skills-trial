mod common;

use common::{Session, err};
use serde_json::json;

fn default_attributes(fifo: bool) -> serde_json::Value {
    json!({
        "VisibilityTimeout": 30,
        "DelaySeconds": 0,
        "RetentionSeconds": 345600,
        "Fifo": fifo,
        "ContentBasedDeduplication": false,
    })
}

#[test]
fn created_queue_has_default_attributes_and_no_messages() {
    let mut s = Session::default();
    s.ok(r#"{"Command": "CreateQueue", "QueueName": "jobs"}"#);
    assert_eq!(
        s.call(r#"{"Command": "GetQueueAttributes", "QueueName": "jobs"}"#),
        json!({
            "ok": true,
            "Attributes": default_attributes(false),
            "Counts": {"Visible": 0, "InFlight": 0, "Delayed": 0},
        })
    );
}

#[test]
fn create_queue_reports_all_given_attributes() {
    let mut s = Session::default();
    s.ok(r#"{"Command": "CreateQueue", "QueueName": "dlq.fifo", "Attributes": {"Fifo": true}}"#);
    s.ok(
        r#"{"Command": "CreateQueue", "QueueName": "q.fifo", "Attributes": {
        "VisibilityTimeout": 5, "DelaySeconds": 6, "RetentionSeconds": 70, "Fifo": true,
        "ContentBasedDeduplication": true, "DeadLetterQueue": "dlq.fifo", "MaxReceiveCount": 2}}"#,
    );
    assert_eq!(
        s.call(r#"{"Command": "GetQueueAttributes", "QueueName": "q.fifo"}"#)["Attributes"],
        json!({
            "VisibilityTimeout": 5,
            "DelaySeconds": 6,
            "RetentionSeconds": 70,
            "Fifo": true,
            "ContentBasedDeduplication": true,
            "DeadLetterQueue": "dlq.fifo",
            "MaxReceiveCount": 2,
        })
    );
}

#[test]
fn create_queue_again_succeeds_only_with_equal_attributes() {
    let mut s = Session::default();
    s.ok(r#"{"Command": "CreateQueue", "QueueName": "q", "Attributes": {"DelaySeconds": 5}}"#);
    s.ok(r#"{"Command": "CreateQueue", "QueueName": "q", "Attributes": {"DelaySeconds": 5, "VisibilityTimeout": 30}}"#);
    assert_eq!(
        s.call(r#"{"Command": "CreateQueue", "QueueName": "q"}"#),
        err("QueueAlreadyExists")
    );
    // Equality is against the current attributes, not the original request.
    s.ok(
        r#"{"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"DelaySeconds": 0}}"#,
    );
    s.ok(r#"{"Command": "CreateQueue", "QueueName": "q"}"#);
}

#[test]
fn create_queue_rule_violations() {
    let mut s = Session::default();
    s.ok(r#"{"Command": "CreateQueue", "QueueName": "std"}"#);
    let cases = [
        (r#"{"QueueName": "a.fifo"}"#, "QueueNameInvalid"),
        (
            r#"{"QueueName": "a", "Attributes": {"Fifo": true}}"#,
            "QueueNameInvalid",
        ),
        (r#"{"QueueName": "a b"}"#, "QueueNameInvalid"),
        (
            r#"{"QueueName": "a", "Attributes": {"ContentBasedDeduplication": true}}"#,
            "InvalidParameterValue",
        ),
        (
            r#"{"QueueName": "a", "Attributes": {"MaxReceiveCount": 3}}"#,
            "InvalidParameterValue",
        ),
        (
            r#"{"QueueName": "a", "Attributes": {"DeadLetterQueue": "std"}}"#,
            "InvalidParameterValue",
        ),
        (
            r#"{"QueueName": "a", "Attributes": {"DeadLetterQueue": "nope", "MaxReceiveCount": 3}}"#,
            "QueueDoesNotExist",
        ),
        (
            r#"{"QueueName": "a.fifo", "Attributes": {"Fifo": true, "DeadLetterQueue": "std", "MaxReceiveCount": 3}}"#,
            "InvalidParameterValue",
        ),
        (
            r#"{"QueueName": "std", "Attributes": {"DelaySeconds": 1}}"#,
            "QueueAlreadyExists",
        ),
    ];
    for (members, code) in cases {
        let line = members.replacen('{', r#"{"Command": "CreateQueue", "#, 1);
        assert_eq!(s.call(&line), err(code), "members {members}");
    }
    assert_eq!(
        s.call(r#"{"Command": "ListQueues"}"#),
        json!({"ok": true, "QueueNames": ["std"]})
    );
}

#[test]
fn queue_limit_is_one_thousand() {
    let mut s = Session::default();
    for i in 0..1000 {
        s.ok(&format!(
            r#"{{"Command": "CreateQueue", "QueueName": "q{i}"}}"#
        ));
    }
    assert_eq!(
        s.call(r#"{"Command": "CreateQueue", "QueueName": "extra"}"#),
        err("LimitExceeded")
    );
    s.ok(r#"{"Command": "CreateQueue", "QueueName": "q0"}"#);
    s.ok(r#"{"Command": "DeleteQueue", "QueueName": "q0"}"#);
    s.ok(r#"{"Command": "CreateQueue", "QueueName": "extra"}"#);
}

#[test]
fn list_queues_sorts_by_bytes_and_filters_by_prefix() {
    let mut s = Session::default();
    for name in ["b", "a_", "aZ", "a1", "a-", "A", "a.fifo"] {
        let fifo = name.ends_with(".fifo");
        s.ok(&format!(
            r#"{{"Command": "CreateQueue", "QueueName": "{name}", "Attributes": {{"Fifo": {fifo}}}}}"#
        ));
    }
    assert_eq!(
        s.call(r#"{"Command": "ListQueues"}"#),
        json!({"ok": true, "QueueNames": ["A", "a-", "a.fifo", "a1", "aZ", "a_", "b"]})
    );
    assert_eq!(
        s.call(r#"{"Command": "ListQueues", "Prefix": "a"}"#),
        json!({"ok": true, "QueueNames": ["a-", "a.fifo", "a1", "aZ", "a_"]})
    );
    assert_eq!(
        s.call(r#"{"Command": "ListQueues", "Prefix": "zz"}"#),
        json!({"ok": true, "QueueNames": []})
    );
    assert_eq!(
        s.call(r#"{"Command": "ListQueues", "Prefix": 1}"#),
        err("InvalidParameterValue")
    );
}

#[test]
fn unknown_queue_is_reported_after_parameter_checks() {
    let mut s = Session::default();
    let cases = [
        (
            r#"{"Command": "GetQueueAttributes", "QueueName": "a b"}"#,
            "QueueDoesNotExist",
        ),
        (
            r#"{"Command": "PurgeQueue", "QueueName": "nope"}"#,
            "QueueDoesNotExist",
        ),
        (
            r#"{"Command": "DeleteQueue", "QueueName": "nope"}"#,
            "QueueDoesNotExist",
        ),
        (
            r#"{"Command": "SetQueueAttributes", "QueueName": "nope", "Attributes": {}}"#,
            "QueueDoesNotExist",
        ),
        (
            r#"{"Command": "SetQueueAttributes", "QueueName": "nope", "Attributes": {"DelaySeconds": -1}}"#,
            "InvalidParameterValue",
        ),
        (
            r#"{"Command": "SetQueueAttributes", "QueueName": "nope"}"#,
            "MissingParameter",
        ),
        (
            r#"{"Command": "SendMessage", "QueueName": "nope", "Body": ""}"#,
            "InvalidParameterValue",
        ),
        (
            r#"{"Command": "DeleteMessage", "QueueName": "nope", "ReceiptHandle": "bogus"}"#,
            "QueueDoesNotExist",
        ),
    ];
    for (line, code) in cases {
        assert_eq!(s.call(line), err(code), "line {line}");
    }
}

#[test]
fn set_queue_attributes_changes_only_given_attributes() {
    let mut s = Session::default();
    s.ok(r#"{"Command": "CreateQueue", "QueueName": "dlq"}"#);
    s.ok(r#"{"Command": "CreateQueue", "QueueName": "q", "Attributes": {"DelaySeconds": 7}}"#);
    s.ok(r#"{"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {
        "VisibilityTimeout": 1, "RetentionSeconds": 60, "DeadLetterQueue": "dlq", "MaxReceiveCount": 4}}"#);
    s.ok(r#"{"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {}}"#);
    assert_eq!(
        s.call(r#"{"Command": "GetQueueAttributes", "QueueName": "q"}"#)["Attributes"],
        json!({
            "VisibilityTimeout": 1,
            "DelaySeconds": 7,
            "RetentionSeconds": 60,
            "Fifo": false,
            "ContentBasedDeduplication": false,
            "DeadLetterQueue": "dlq",
            "MaxReceiveCount": 4,
        })
    );
}

#[test]
fn set_queue_attributes_rule_violations() {
    let mut s = Session::default();
    s.ok(r#"{"Command": "CreateQueue", "QueueName": "q"}"#);
    s.ok(r#"{"Command": "CreateQueue", "QueueName": "f.fifo", "Attributes": {"Fifo": true}}"#);
    let cases = [
        (r#"{"Fifo": false}"#, "UnsupportedOperation"),
        (r#"{"Fifo": "x"}"#, "InvalidParameterValue"),
        (
            r#"{"ContentBasedDeduplication": true}"#,
            "InvalidParameterValue",
        ),
        (r#"{"DeadLetterQueue": "q"}"#, "InvalidParameterValue"),
        (
            r#"{"DeadLetterQueue": "nope", "MaxReceiveCount": 1}"#,
            "QueueDoesNotExist",
        ),
        (
            r#"{"DeadLetterQueue": "q", "MaxReceiveCount": 1}"#,
            "InvalidParameterValue",
        ),
        (
            r#"{"DeadLetterQueue": "f.fifo", "MaxReceiveCount": 1}"#,
            "InvalidParameterValue",
        ),
        (
            r#"{"VisibilityTimeout": 5, "Fifo": true}"#,
            "UnsupportedOperation",
        ),
    ];
    for (attributes, code) in cases {
        let line = format!(
            r#"{{"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {attributes}}}"#
        );
        assert_eq!(s.call(&line), err(code), "attributes {attributes}");
    }
    // A failed command changes nothing, not even the attributes that were valid.
    assert_eq!(
        s.call(r#"{"Command": "GetQueueAttributes", "QueueName": "q"}"#)["Attributes"],
        default_attributes(false)
    );
    s.ok(r#"{"Command": "SetQueueAttributes", "QueueName": "f.fifo", "Attributes": {"ContentBasedDeduplication": true}}"#);
}

#[test]
fn deleting_a_dead_letter_queue_clears_configs_that_name_it() {
    let mut s = Session::default();
    s.ok(r#"{"Command": "CreateQueue", "QueueName": "dlq"}"#);
    s.ok(r#"{"Command": "CreateQueue", "QueueName": "q", "Attributes": {"DeadLetterQueue": "dlq", "MaxReceiveCount": 1}}"#);
    s.ok(r#"{"Command": "DeleteQueue", "QueueName": "dlq"}"#);
    assert_eq!(
        s.call(r#"{"Command": "GetQueueAttributes", "QueueName": "q"}"#)["Attributes"],
        default_attributes(false)
    );
}

#[test]
fn recreated_queue_shares_nothing_with_the_deleted_one() {
    let mut s = Session::default();
    s.ok(r#"{"Command": "CreateQueue", "QueueName": "q"}"#);
    s.call(r#"{"Command": "SendMessage", "QueueName": "q", "Body": "old"}"#);
    let handle = s.call(r#"{"Command": "ReceiveMessage", "QueueName": "q"}"#)["Messages"][0]["ReceiptHandle"].clone();
    s.ok(r#"{"Command": "DeleteQueue", "QueueName": "q"}"#);
    s.ok(r#"{"Command": "CreateQueue", "QueueName": "q"}"#);
    assert_eq!(
        s.call(r#"{"Command": "GetQueueAttributes", "QueueName": "q"}"#)["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
    let line =
        format!(r#"{{"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": {handle}}}"#);
    assert_eq!(s.call(&line), err("ReceiptHandleExpired"));
}

#[test]
fn tick_advances_the_clock() {
    let mut s = Session::default();
    assert_eq!(
        s.call(r#"{"Command": "Tick", "Seconds": 0}"#),
        json!({"ok": true, "Now": 0})
    );
    assert_eq!(
        s.call(r#"{"Command": "Tick", "Seconds": 5}"#),
        json!({"ok": true, "Now": 5})
    );
    assert_eq!(
        s.call(r#"{"Command": "Tick", "Seconds": 1209600}"#),
        json!({"ok": true, "Now": 1209605})
    );
    assert_eq!(
        s.call(r#"{"Command": "Tick", "Seconds": 1209601}"#),
        err("InvalidParameterValue")
    );
}
