mod common;

use common::run;
use serde_json::{Value, json};

const CREATE_DLQ: &str = r#"{"Command": "CreateQueue", "QueueName": "dlq"}"#;
const CREATE_Q: &str = r#"{"Command": "CreateQueue", "QueueName": "q", "Attributes": {"VisibilityTimeout": 0, "DeadLetterQueue": "dlq", "MaxReceiveCount": 2}}"#;
const SEND: &str = r#"{"Command": "SendMessage", "QueueName": "q", "Body": "a"}"#;
const RECEIVE: &str = r#"{"Command": "ReceiveMessage", "QueueName": "q"}"#;
const RECEIVE_DLQ: &str = r#"{"Command": "ReceiveMessage", "QueueName": "dlq"}"#;

fn bodies(response: &Value) -> Vec<&str> {
    response["Messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["Body"].as_str().unwrap())
        .collect()
}

#[test]
fn a_message_moves_after_max_receive_count_deliveries_and_starts_over() {
    let out = run(&[
        CREATE_DLQ,
        CREATE_Q,
        SEND,
        r#"{"Command": "Tick", "Seconds": 7}"#,
        RECEIVE,
        RECEIVE,
        RECEIVE,
        r#"{"Command": "GetQueueAttributes", "QueueName": "q"}"#,
        r#"{"Command": "Tick", "Seconds": 3}"#,
        RECEIVE_DLQ,
    ]);
    assert_eq!(out[4]["Messages"][0]["ReceiveCount"], 1);
    assert_eq!(out[5]["Messages"][0]["ReceiveCount"], 2);
    assert_eq!(out[6], json!({"ok": true, "Messages": []}));
    assert_eq!(
        out[7]["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
    // Same id and send time, a fresh receive count and first-receive time,
    // and the next receipt handle of the process.
    assert_eq!(
        out[9]["Messages"],
        json!([{
            "MessageId": "m-1", "ReceiptHandle": "r-3", "Body": "a",
            "ReceiveCount": 1, "SentAt": 0, "FirstReceivedAt": 10
        }])
    );
}

#[test]
fn moved_messages_do_not_count_toward_max_messages() {
    let out = run(&[
        CREATE_DLQ,
        r#"{"Command": "CreateQueue", "QueueName": "q", "Attributes": {"VisibilityTimeout": 0, "DeadLetterQueue": "dlq", "MaxReceiveCount": 1}}"#,
        SEND,
        r#"{"Command": "SendMessage", "QueueName": "q", "Body": "b"}"#,
        RECEIVE,
        RECEIVE,
        RECEIVE_DLQ,
    ]);
    assert_eq!(bodies(&out[4]), ["a"]);
    assert_eq!(bodies(&out[5]), ["b"]);
    assert_eq!(bodies(&out[6]), ["a"]);
}

#[test]
fn a_new_dead_letter_configuration_applies_from_the_next_receive() {
    let out = run(&[
        CREATE_DLQ,
        r#"{"Command": "CreateQueue", "QueueName": "q", "Attributes": {"VisibilityTimeout": 0}}"#,
        SEND,
        RECEIVE,
        RECEIVE,
        RECEIVE,
        r#"{"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"DeadLetterQueue": "dlq", "MaxReceiveCount": 3}}"#,
        RECEIVE,
        RECEIVE_DLQ,
    ]);
    assert_eq!(out[5]["Messages"][0]["ReceiveCount"], 3);
    assert_eq!(out[7], json!({"ok": true, "Messages": []}));
    assert_eq!(bodies(&out[8]), ["a"]);
}

#[test]
fn retention_of_the_dead_letter_queue_applies_on_arrival() {
    let out = run(&[
        r#"{"Command": "CreateQueue", "QueueName": "dlq", "Attributes": {"RetentionSeconds": 60}}"#,
        r#"{"Command": "CreateQueue", "QueueName": "q", "Attributes": {"VisibilityTimeout": 0, "DeadLetterQueue": "dlq", "MaxReceiveCount": 1}}"#,
        SEND,
        RECEIVE,
        r#"{"Command": "Tick", "Seconds": 60}"#,
        RECEIVE,
        r#"{"Command": "GetQueueAttributes", "QueueName": "dlq"}"#,
    ]);
    assert_eq!(out[5], json!({"ok": true, "Messages": []}));
    assert_eq!(
        out[6]["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn fifo_dead_letter_moves_unblock_the_group_and_take_new_sequence_numbers() {
    let send = |body: &str, id: &str| {
        json!({"Command": "SendMessage", "QueueName": "q.fifo", "Body": body,
               "GroupId": "g", "DeduplicationId": id})
        .to_string()
    };
    let (a1, a2) = (send("a1", "1"), send("a2", "2"));
    let out = run(&[
        r#"{"Command": "CreateQueue", "QueueName": "dlq.fifo", "Attributes": {"Fifo": true}}"#,
        r#"{"Command": "CreateQueue", "QueueName": "q.fifo", "Attributes": {"Fifo": true, "VisibilityTimeout": 0, "DeadLetterQueue": "dlq.fifo", "MaxReceiveCount": 1}}"#,
        &a1,
        &a2,
        r#"{"Command": "ReceiveMessage", "QueueName": "q.fifo"}"#,
        r#"{"Command": "ReceiveMessage", "QueueName": "q.fifo"}"#,
        r#"{"Command": "ReceiveMessage", "QueueName": "dlq.fifo"}"#,
    ]);
    assert_eq!(bodies(&out[4]), ["a1"]);
    // a1 leaves for the dead-letter queue, so a2 is first in its group.
    assert_eq!(bodies(&out[5]), ["a2"]);
    assert_eq!(out[5]["Messages"][0]["SequenceNumber"], 2);
    assert_eq!(out[6]["Messages"][0]["MessageId"], "m-1");
    assert_eq!(out[6]["Messages"][0]["GroupId"], "g");
    assert_eq!(out[6]["Messages"][0]["SequenceNumber"], 1);
}

#[test]
fn the_move_does_not_record_a_deduplication_id() {
    let out = run(&[
        r#"{"Command": "CreateQueue", "QueueName": "dlq.fifo", "Attributes": {"Fifo": true}}"#,
        r#"{"Command": "CreateQueue", "QueueName": "q.fifo", "Attributes": {"Fifo": true, "VisibilityTimeout": 0, "DeadLetterQueue": "dlq.fifo", "MaxReceiveCount": 1}}"#,
        r#"{"Command": "SendMessage", "QueueName": "q.fifo", "Body": "a", "GroupId": "g", "DeduplicationId": "1"}"#,
        r#"{"Command": "ReceiveMessage", "QueueName": "q.fifo"}"#,
        r#"{"Command": "ReceiveMessage", "QueueName": "q.fifo"}"#,
        r#"{"Command": "SendMessage", "QueueName": "dlq.fifo", "Body": "b", "GroupId": "h", "DeduplicationId": "1"}"#,
        r#"{"Command": "ReceiveMessage", "QueueName": "dlq.fifo", "MaxMessages": 10}"#,
    ]);
    // The moved message took sequence number 1; a duplicate of its old
    // deduplication id would have answered with m-1 instead of m-2.
    assert_eq!(
        out[5],
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2})
    );
    assert_eq!(bodies(&out[6]), ["a", "b"]);
}
