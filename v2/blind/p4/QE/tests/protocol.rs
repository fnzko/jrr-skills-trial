//! End-to-end checks of the protocol. Each test is one session: the expected
//! responses come from SPEC.md, not from running the code.

use miniq::protocol::serve;
use serde_json::{json, Value};

/// Runs the lines through one fresh session and returns each response parsed.
fn session<S: AsRef<str>>(lines: &[S]) -> Vec<Value> {
    let mut input = String::new();
    for line in lines {
        input.push_str(line.as_ref());
        input.push('\n');
    }
    let mut output = Vec::new();
    serve(input.as_bytes(), &mut output).expect("writing to a Vec does not fail");
    String::from_utf8(output)
        .expect("responses are UTF-8")
        .lines()
        .map(|line| serde_json::from_str(line).expect("each response is JSON"))
        .collect()
}

#[test]
fn fifo_receive_delivers_at_most_one_message_per_group_in_arrival_order() {
    let responses = session(&[
        r#"{"Command": "CreateQueue", "QueueName": "jobs.fifo", "Attributes": {"Fifo": true}}"#,
        r#"{"Command": "SendMessage", "QueueName": "jobs.fifo", "Body": "a1", "GroupId": "a", "DeduplicationId": "1"}"#,
        r#"{"Command": "SendMessage", "QueueName": "jobs.fifo", "Body": "a2", "GroupId": "a", "DeduplicationId": "2"}"#,
        r#"{"Command": "SendMessage", "QueueName": "jobs.fifo", "Body": "b1", "GroupId": "b", "DeduplicationId": "3"}"#,
        r#"{"Command": "ReceiveMessage", "QueueName": "jobs.fifo", "MaxMessages": 10}"#,
    ]);
    assert_eq!(
        responses[1],
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
    assert_eq!(
        responses[2],
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2})
    );
    assert_eq!(
        responses[3],
        json!({"ok": true, "MessageId": "m-3", "SequenceNumber": 3})
    );
    assert_eq!(
        responses[4],
        json!({
            "ok": true,
            "Messages": [
                {
                    "MessageId": "m-1",
                    "ReceiptHandle": "r-1",
                    "Body": "a1",
                    "ReceiveCount": 1,
                    "SentAt": 0,
                    "FirstReceivedAt": 0,
                    "GroupId": "a",
                    "SequenceNumber": 1
                },
                {
                    "MessageId": "m-3",
                    "ReceiptHandle": "r-2",
                    "Body": "b1",
                    "ReceiveCount": 1,
                    "SentAt": 0,
                    "FirstReceivedAt": 0,
                    "GroupId": "b",
                    "SequenceNumber": 3
                }
            ]
        })
    );
}

#[test]
fn a_duplicate_is_reported_inside_the_window_and_accepted_after_it() {
    let responses = session(&[
        r#"{"Command": "CreateQueue", "QueueName": "f.fifo", "Attributes": {"Fifo": true}}"#,
        r#"{"Command": "SendMessage", "QueueName": "f.fifo", "Body": "x", "GroupId": "g", "DeduplicationId": "d"}"#,
        r#"{"Command": "Tick", "Seconds": 299}"#,
        r#"{"Command": "SendMessage", "QueueName": "f.fifo", "Body": "x", "GroupId": "g", "DeduplicationId": "d"}"#,
        r#"{"Command": "Tick", "Seconds": 1}"#,
        r#"{"Command": "SendMessage", "QueueName": "f.fifo", "Body": "x", "GroupId": "g", "DeduplicationId": "d"}"#,
    ]);
    assert_eq!(
        responses[3],
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
    assert_eq!(
        responses[5],
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2})
    );
}

#[test]
fn delayed_messages_appear_only_when_the_delay_has_passed() {
    let responses = session(&[
        r#"{"Command": "CreateQueue", "QueueName": "d"}"#,
        r#"{"Command": "SendMessage", "QueueName": "d", "Body": "later", "DelaySeconds": 10}"#,
        r#"{"Command": "GetQueueAttributes", "QueueName": "d"}"#,
        r#"{"Command": "Tick", "Seconds": 9}"#,
        r#"{"Command": "ReceiveMessage", "QueueName": "d"}"#,
        r#"{"Command": "Tick", "Seconds": 1}"#,
        r#"{"Command": "ReceiveMessage", "QueueName": "d"}"#,
    ]);
    assert_eq!(
        responses[2]["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 1})
    );
    assert_eq!(responses[4], json!({"ok": true, "Messages": []}));
    assert_eq!(responses[6]["Messages"][0]["SentAt"], 0);
    assert_eq!(responses[6]["Messages"][0]["MessageId"], "m-1");
}

#[test]
fn a_message_received_too_often_moves_to_the_fifo_dead_letter_queue() {
    let responses = session(&[
        r#"{"Command": "CreateQueue", "QueueName": "dead.fifo", "Attributes": {"Fifo": true}}"#,
        r#"{"Command": "CreateQueue", "QueueName": "src.fifo", "Attributes": {"Fifo": true, "DeadLetterQueue": "dead.fifo", "MaxReceiveCount": 1, "VisibilityTimeout": 0}}"#,
        r#"{"Command": "SendMessage", "QueueName": "src.fifo", "Body": "poison", "GroupId": "g", "DeduplicationId": "p"}"#,
        r#"{"Command": "ReceiveMessage", "QueueName": "src.fifo"}"#,
        r#"{"Command": "ReceiveMessage", "QueueName": "src.fifo"}"#,
        r#"{"Command": "ReceiveMessage", "QueueName": "dead.fifo"}"#,
        r#"{"Command": "DeleteMessage", "QueueName": "src.fifo", "ReceiptHandle": "r-1"}"#,
    ]);
    assert_eq!(responses[4], json!({"ok": true, "Messages": []}));
    assert_eq!(
        responses[5],
        json!({
            "ok": true,
            "Messages": [{
                "MessageId": "m-1",
                "ReceiptHandle": "r-2",
                "Body": "poison",
                "ReceiveCount": 1,
                "SentAt": 0,
                "FirstReceivedAt": 0,
                "GroupId": "g",
                "SequenceNumber": 1
            }]
        })
    );
    assert_eq!(responses[6]["error"]["code"], "ReceiptHandleExpired");
}

#[test]
fn lowering_retention_removes_messages_at_once() {
    let responses = session(&[
        r#"{"Command": "CreateQueue", "QueueName": "q"}"#,
        r#"{"Command": "SendMessage", "QueueName": "q", "Body": "old"}"#,
        r#"{"Command": "Tick", "Seconds": 100}"#,
        r#"{"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"RetentionSeconds": 60}}"#,
        r#"{"Command": "GetQueueAttributes", "QueueName": "q"}"#,
    ]);
    assert_eq!(responses[3], json!({"ok": true}));
    assert_eq!(
        responses[4]["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn a_handle_stays_expired_after_its_queue_is_deleted_and_recreated() {
    let responses = session(&[
        r#"{"Command": "CreateQueue", "QueueName": "q"}"#,
        r#"{"Command": "SendMessage", "QueueName": "q", "Body": "first"}"#,
        r#"{"Command": "ReceiveMessage", "QueueName": "q"}"#,
        r#"{"Command": "DeleteQueue", "QueueName": "q"}"#,
        r#"{"Command": "CreateQueue", "QueueName": "q"}"#,
        r#"{"Command": "SendMessage", "QueueName": "q", "Body": "second"}"#,
        r#"{"Command": "ReceiveMessage", "QueueName": "q"}"#,
        r#"{"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": "r-1"}"#,
        r#"{"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": "r-2"}"#,
    ]);
    assert_eq!(responses[5], json!({"ok": true, "MessageId": "m-2"}));
    assert_eq!(responses[6]["Messages"][0]["ReceiptHandle"], "r-2");
    assert_eq!(responses[7]["error"]["code"], "ReceiptHandleExpired");
    assert_eq!(responses[8], json!({"ok": true}));
}

#[test]
fn a_zero_visibility_timeout_makes_the_handle_unusable_at_once() {
    let responses = session(&[
        r#"{"Command": "CreateQueue", "QueueName": "q"}"#,
        r#"{"Command": "SendMessage", "QueueName": "q", "Body": "x"}"#,
        r#"{"Command": "ReceiveMessage", "QueueName": "q", "VisibilityTimeout": 0}"#,
        r#"{"Command": "ChangeMessageVisibility", "QueueName": "q", "ReceiptHandle": "r-1", "VisibilityTimeout": 30}"#,
    ]);
    assert_eq!(responses[3]["error"]["code"], "ReceiptHandleExpired");
}

#[test]
fn purge_removes_messages_but_not_deduplication_history() {
    let responses = session(&[
        r#"{"Command": "CreateQueue", "QueueName": "f.fifo", "Attributes": {"Fifo": true}}"#,
        r#"{"Command": "SendMessage", "QueueName": "f.fifo", "Body": "x", "GroupId": "g", "DeduplicationId": "d"}"#,
        r#"{"Command": "PurgeQueue", "QueueName": "f.fifo"}"#,
        r#"{"Command": "SendMessage", "QueueName": "f.fifo", "Body": "x", "GroupId": "g", "DeduplicationId": "d"}"#,
    ]);
    assert_eq!(
        responses[3],
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
}

#[test]
fn missing_parameters_win_over_bad_values_and_missing_queues() {
    let responses = session(&[
        r#"{"Command": "CreateQueue", "QueueName": "y"}"#,
        r#"{"Command": "SendMessage", "QueueName": "nope"}"#,
        r#"{"Command": "SendMessage", "QueueName": "nope", "Body": "x"}"#,
        r#"{"Command": "CreateQueue", "QueueName": "a b", "Attributes": {"Bogus": 1}}"#,
        r#"{"Command": "CreateQueue", "QueueName": "x", "Attributes": {"Bogus": 1}}"#,
        r#"{"Command": "DeleteMessage", "QueueName": "nope", "ReceiptHandle": "r-1"}"#,
        r#"{"Command": "DeleteMessage", "QueueName": "y", "ReceiptHandle": "zz"}"#,
    ]);
    let codes: Vec<&str> = responses[1..]
        .iter()
        .map(|response| response["error"]["code"].as_str().unwrap())
        .collect();
    assert_eq!(
        codes,
        [
            "MissingParameter",
            "QueueDoesNotExist",
            "QueueNameInvalid",
            "InvalidParameterValue",
            "QueueDoesNotExist",
            "ReceiptHandleInvalid",
        ]
    );
}

#[test]
fn a_session_stops_creating_queues_at_1000() {
    let mut lines: Vec<String> = (0..1000)
        .map(|n| format!(r#"{{"Command": "CreateQueue", "QueueName": "q{n}"}}"#))
        .collect();
    lines.push(r#"{"Command": "CreateQueue", "QueueName": "extra"}"#.to_owned());
    let responses = session(&lines);
    assert_eq!(responses[999], json!({"ok": true}));
    assert_eq!(responses[1000]["error"]["code"], "LimitExceeded");
}

#[test]
fn a_queue_refuses_the_100001st_message() {
    let mut lines = vec![r#"{"Command": "CreateQueue", "QueueName": "big"}"#.to_owned()];
    lines
        .extend((0..100_001).map(|n| {
            format!(r#"{{"Command": "SendMessage", "QueueName": "big", "Body": "{n}"}}"#)
        }));
    let responses = session(&lines);
    assert_eq!(
        responses[100_000],
        json!({"ok": true, "MessageId": "m-100000"})
    );
    assert_eq!(responses[100_001]["error"]["code"], "LimitExceeded");
}
