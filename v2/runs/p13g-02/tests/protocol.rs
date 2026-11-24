//! Whole-protocol scenarios: command lines in, response objects out.

use miniq::broker::Broker;
use serde_json::{Value, json};

fn session(lines: &[&str]) -> Vec<Value> {
    let mut broker = Broker::default();
    lines
        .iter()
        .map(|line| {
            let response = broker.respond(line.as_bytes());
            serde_json::from_str(&response).expect("every response is one JSON value")
        })
        .collect()
}

fn ok() -> Value {
    json!({"ok": true})
}

fn error_code(response: &Value) -> &str {
    assert_eq!(response["ok"], json!(false), "response {response}");
    response["error"]["code"]
        .as_str()
        .expect("error has a code")
}

fn codes(lines: &[&str]) -> Vec<String> {
    session(lines)
        .iter()
        .map(|r| error_code(r).to_owned())
        .collect()
}

#[test]
fn created_queue_reports_defaults_and_counts() {
    let out = session(&[
        r#"{"Command": "CreateQueue", "QueueName": "jobs"}"#,
        r#"{"Command": "GetQueueAttributes", "QueueName": "jobs"}"#,
    ]);
    assert_eq!(out[0], ok());
    assert_eq!(
        out[1],
        json!({
            "ok": true,
            "Attributes": {
                "VisibilityTimeout": 30, "DelaySeconds": 0, "RetentionSeconds": 345600,
                "Fifo": false, "ContentBasedDeduplication": false
            },
            "Counts": {"Visible": 0, "InFlight": 0, "Delayed": 0}
        })
    );
}

#[test]
fn create_queue_is_idempotent_only_for_equal_attributes() {
    let out = session(&[
        r#"{"Command": "CreateQueue", "QueueName": "q", "Attributes": {"DelaySeconds": 5}}"#,
        r#"{"Command": "CreateQueue", "QueueName": "q", "Attributes": {"DelaySeconds": 5, "Fifo": null}}"#,
        r#"{"Command": "CreateQueue", "QueueName": "q"}"#,
    ]);
    assert_eq!(out[1], ok());
    assert_eq!(error_code(&out[2]), "QueueAlreadyExists");
}

#[test]
fn create_queue_rules_apply_in_the_written_order() {
    let attributes = |name: &str, attrs: &str| {
        format!(r#"{{"Command": "CreateQueue", "QueueName": "{name}", "Attributes": {attrs}}}"#)
    };
    let cases = [
        (attributes("a.fifo", "{}"), "QueueNameInvalid"),
        (attributes("a", r#"{"Fifo": true}"#), "QueueNameInvalid"),
        (
            attributes("a", r#"{"ContentBasedDeduplication": true}"#),
            "InvalidParameterValue",
        ),
        (
            attributes("a", r#"{"MaxReceiveCount": 2}"#),
            "InvalidParameterValue",
        ),
        (
            attributes("a", r#"{"DeadLetterQueue": "x", "MaxReceiveCount": 2}"#),
            "QueueDoesNotExist",
        ),
    ];
    for (line, code) in cases {
        assert_eq!(codes(&[&line]), [code], "line {line}");
    }
}

#[test]
fn dead_letter_queue_must_have_the_same_type() {
    let out = session(&[
        r#"{"Command": "CreateQueue", "QueueName": "d.fifo", "Attributes": {"Fifo": true}}"#,
        r#"{"Command": "CreateQueue", "QueueName": "q", "Attributes": {"DeadLetterQueue": "d.fifo", "MaxReceiveCount": 2}}"#,
        r#"{"Command": "CreateQueue", "QueueName": "s"}"#,
        r#"{"Command": "SetQueueAttributes", "QueueName": "s", "Attributes": {"DeadLetterQueue": "s", "MaxReceiveCount": 2}}"#,
        r#"{"Command": "SetQueueAttributes", "QueueName": "s", "Attributes": {"DeadLetterQueue": "d.fifo", "MaxReceiveCount": 2}}"#,
    ]);
    assert_eq!(error_code(&out[1]), "InvalidParameterValue");
    assert_eq!(error_code(&out[3]), "InvalidParameterValue");
    assert_eq!(error_code(&out[4]), "InvalidParameterValue");
}

#[test]
fn set_queue_attributes_rules() {
    let out = session(&[
        r#"{"Command": "CreateQueue", "QueueName": "q"}"#,
        r#"{"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"Fifo": false}}"#,
        r#"{"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"ContentBasedDeduplication": true}}"#,
        r#"{"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"MaxReceiveCount": 3}}"#,
        r#"{"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"DeadLetterQueue": "nope", "MaxReceiveCount": 3}}"#,
        r#"{"Command": "SetQueueAttributes", "QueueName": "missing", "Attributes": {}}"#,
        r#"{"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"VisibilityTimeout": 7}}"#,
        r#"{"Command": "GetQueueAttributes", "QueueName": "q"}"#,
    ]);
    let found: Vec<_> = out[1..6].iter().map(error_code).collect();
    assert_eq!(
        found,
        [
            "UnsupportedOperation",
            "InvalidParameterValue",
            "InvalidParameterValue",
            "QueueDoesNotExist",
            "QueueDoesNotExist"
        ]
    );
    assert_eq!(out[6], ok());
    assert_eq!(out[7]["Attributes"]["VisibilityTimeout"], json!(7));
}

#[test]
fn list_queues_sorts_by_bytes_and_filters_by_prefix() {
    let creates: Vec<String> = ["b", "a.fifo", "A", "_x", "a-1", "a1"]
        .iter()
        .map(|name| {
            let fifo = name.ends_with(".fifo");
            format!(r#"{{"Command": "CreateQueue", "QueueName": "{name}", "Attributes": {{"Fifo": {fifo}}}}}"#)
        })
        .collect();
    let mut lines: Vec<&str> = creates.iter().map(String::as_str).collect();
    lines.push(r#"{"Command": "ListQueues"}"#);
    lines.push(r#"{"Command": "ListQueues", "Prefix": "a"}"#);
    lines.push(r#"{"Command": "ListQueues", "Prefix": ""}"#);
    let out = session(&lines);
    assert_eq!(
        out[6],
        json!({"ok": true, "QueueNames": ["A", "_x", "a-1", "a.fifo", "a1", "b"]})
    );
    assert_eq!(
        out[7],
        json!({"ok": true, "QueueNames": ["a-1", "a.fifo", "a1"]})
    );
    assert_eq!(out[8], out[6]);
}

#[test]
fn send_receive_delete_round_trip() {
    let out = session(&[
        r#"{"Command": "CreateQueue", "QueueName": "q"}"#,
        r#"{"Command": "SendMessage", "QueueName": "q", "Body": "hello"}"#,
        r#"{"Command": "Tick", "Seconds": 4}"#,
        r#"{"Command": "ReceiveMessage", "QueueName": "q"}"#,
        r#"{"Command": "ReceiveMessage", "QueueName": "q"}"#,
        r#"{"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": "r-1"}"#,
        r#"{"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": "r-1"}"#,
        r#"{"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": "r-2"}"#,
        r#"{"Command": "DeleteMessage", "QueueName": "nope", "ReceiptHandle": "zzz"}"#,
    ]);
    assert_eq!(out[1], json!({"ok": true, "MessageId": "m-1"}));
    assert_eq!(out[2], json!({"ok": true, "Now": 4}));
    assert_eq!(
        out[3],
        json!({"ok": true, "Messages": [{
            "MessageId": "m-1", "ReceiptHandle": "r-1", "Body": "hello",
            "ReceiveCount": 1, "SentAt": 0, "FirstReceivedAt": 4
        }]})
    );
    assert_eq!(out[4], json!({"ok": true, "Messages": []}));
    assert_eq!(out[5], ok());
    assert_eq!(error_code(&out[6]), "ReceiptHandleExpired");
    assert_eq!(error_code(&out[7]), "ReceiptHandleInvalid");
    assert_eq!(error_code(&out[8]), "QueueDoesNotExist");
}

#[test]
fn fifo_send_returns_sequence_numbers_and_deduplicates() {
    let out = session(&[
        r#"{"Command": "CreateQueue", "QueueName": "q.fifo", "Attributes": {"Fifo": true}}"#,
        r#"{"Command": "SendMessage", "QueueName": "q.fifo", "Body": "a", "GroupId": "g", "DeduplicationId": "d"}"#,
        r#"{"Command": "SendMessage", "QueueName": "q.fifo", "Body": "b", "GroupId": "h", "DeduplicationId": "d"}"#,
        r#"{"Command": "SendMessage", "QueueName": "q.fifo", "Body": "c", "GroupId": "g", "DeduplicationId": "e"}"#,
        r#"{"Command": "ReceiveMessage", "QueueName": "q.fifo", "MaxMessages": 10}"#,
    ]);
    assert_eq!(
        out[1],
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
    assert_eq!(out[2], out[1]);
    assert_eq!(
        out[3],
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2})
    );
    assert_eq!(
        out[4],
        json!({"ok": true, "Messages": [{
            "MessageId": "m-1", "ReceiptHandle": "r-1", "Body": "a", "ReceiveCount": 1,
            "SentAt": 0, "FirstReceivedAt": 0, "GroupId": "g", "SequenceNumber": 1
        }]})
    );
}

#[test]
fn received_too_often_moves_to_the_dead_letter_queue() {
    let out = session(&[
        r#"{"Command": "CreateQueue", "QueueName": "dlq"}"#,
        r#"{"Command": "CreateQueue", "QueueName": "q", "Attributes": {"VisibilityTimeout": 0, "DeadLetterQueue": "dlq", "MaxReceiveCount": 2}}"#,
        r#"{"Command": "SendMessage", "QueueName": "q", "Body": "x"}"#,
        r#"{"Command": "ReceiveMessage", "QueueName": "q"}"#,
        r#"{"Command": "ReceiveMessage", "QueueName": "q"}"#,
        r#"{"Command": "ReceiveMessage", "QueueName": "q"}"#,
        r#"{"Command": "ReceiveMessage", "QueueName": "dlq"}"#,
        r#"{"Command": "DeleteQueue", "QueueName": "dlq"}"#,
        r#"{"Command": "GetQueueAttributes", "QueueName": "q"}"#,
    ]);
    assert_eq!(out[5], json!({"ok": true, "Messages": []}));
    assert_eq!(
        out[6],
        json!({"ok": true, "Messages": [{
            "MessageId": "m-1", "ReceiptHandle": "r-3", "Body": "x", "ReceiveCount": 1,
            "SentAt": 0, "FirstReceivedAt": 0
        }]})
    );
    assert_eq!(
        out[8]["Attributes"],
        json!({
            "VisibilityTimeout": 0, "DelaySeconds": 0, "RetentionSeconds": 345600,
            "Fifo": false, "ContentBasedDeduplication": false
        })
    );
}

#[test]
fn retention_drops_messages_when_lowered_and_keeps_them_gone() {
    let out = session(&[
        r#"{"Command": "CreateQueue", "QueueName": "q"}"#,
        r#"{"Command": "SendMessage", "QueueName": "q", "Body": "x"}"#,
        r#"{"Command": "Tick", "Seconds": 100}"#,
        r#"{"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"RetentionSeconds": 60}}"#,
        r#"{"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"RetentionSeconds": 600}}"#,
        r#"{"Command": "GetQueueAttributes", "QueueName": "q"}"#,
    ]);
    assert_eq!(
        out[5]["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn deleted_queue_name_can_be_reused_with_a_clean_slate() {
    let out = session(&[
        r#"{"Command": "CreateQueue", "QueueName": "q"}"#,
        r#"{"Command": "SendMessage", "QueueName": "q", "Body": "x"}"#,
        r#"{"Command": "ReceiveMessage", "QueueName": "q"}"#,
        r#"{"Command": "DeleteQueue", "QueueName": "q"}"#,
        r#"{"Command": "CreateQueue", "QueueName": "q"}"#,
        r#"{"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": "r-1"}"#,
        r#"{"Command": "PurgeQueue", "QueueName": "missing"}"#,
    ]);
    assert_eq!(error_code(&out[5]), "ReceiptHandleExpired");
    assert_eq!(error_code(&out[6]), "QueueDoesNotExist");
}

#[test]
fn change_visibility_to_zero_makes_the_message_visible_at_once() {
    let out = session(&[
        r#"{"Command": "CreateQueue", "QueueName": "q"}"#,
        r#"{"Command": "SendMessage", "QueueName": "q", "Body": "x"}"#,
        r#"{"Command": "ReceiveMessage", "QueueName": "q"}"#,
        r#"{"Command": "ChangeMessageVisibility", "QueueName": "q", "ReceiptHandle": "r-1", "VisibilityTimeout": 0}"#,
        r#"{"Command": "GetQueueAttributes", "QueueName": "q"}"#,
    ]);
    assert_eq!(out[3], ok());
    assert_eq!(
        out[4]["Counts"],
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn failed_requests_use_the_first_failing_check() {
    let found = codes(&[
        "",
        r#"{"Command": "PurgeQueue"}"#,
        r#"{"Command": "PurgeQueue", "QueueName": 5}"#,
        r#"{"Command": "PurgeQueue", "QueueName": "a b"}"#,
        r#"{"Command": "SendMessage", "QueueName": "missing", "Body": ""}"#,
        r#"{"Command": "Tick", "Seconds": 1.5}"#,
        r#"{"Command": "CreateQueue", "QueueName": "x.fifo"}"#,
    ]);
    assert_eq!(
        found,
        [
            "InvalidRequest",
            "MissingParameter",
            "InvalidParameterValue",
            "QueueDoesNotExist",
            "InvalidParameterValue",
            "InvalidParameterValue",
            "QueueNameInvalid"
        ]
    );
}

#[test]
fn thousand_queues_is_the_limit() {
    let mut broker = Broker::default();
    for n in 0..1000 {
        let line = format!(r#"{{"Command": "CreateQueue", "QueueName": "q{n}"}}"#);
        assert_eq!(broker.respond(line.as_bytes()), r#"{"ok":true}"#);
    }
    let again = broker.respond(br#"{"Command": "CreateQueue", "QueueName": "q5"}"#);
    assert_eq!(again, r#"{"ok":true}"#);
    let over: Value = serde_json::from_str(
        &broker.respond(br#"{"Command": "CreateQueue", "QueueName": "extra"}"#),
    )
    .unwrap();
    assert_eq!(error_code(&over), "LimitExceeded");
}
