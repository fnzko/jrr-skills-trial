mod common;

use common::{assert_error, session};
use serde_json::{json, Value};

fn send(queue: &str, body: &str) -> String {
    json!({"Command": "SendMessage", "QueueName": queue, "Body": body}).to_string()
}

fn send_fifo(queue: &str, body: &str, group: &str) -> String {
    json!({"Command": "SendMessage", "QueueName": queue, "Body": body, "GroupId": group})
        .to_string()
}

fn receive(queue: &str) -> String {
    json!({"Command": "ReceiveMessage", "QueueName": queue}).to_string()
}

fn receive_one(queue: &str) -> String {
    json!({"Command": "ReceiveMessage", "QueueName": queue, "MaxMessages": 1}).to_string()
}

fn receive_all(queue: &str) -> String {
    json!({"Command": "ReceiveMessage", "QueueName": queue, "MaxMessages": 10}).to_string()
}

fn tick(seconds: u32) -> String {
    json!({"Command": "Tick", "Seconds": seconds}).to_string()
}

fn counts(queue: &str) -> String {
    json!({"Command": "GetQueueAttributes", "QueueName": queue}).to_string()
}

fn delete(queue: &str, receipt: &str) -> String {
    json!({"Command": "DeleteMessage", "QueueName": queue, "ReceiptHandle": receipt}).to_string()
}

fn bodies(response: &Value) -> Vec<&str> {
    response["Messages"]
        .as_array()
        .expect("Messages is an array")
        .iter()
        .map(|message| message["Body"].as_str().unwrap())
        .collect()
}

#[test]
fn message_moves_to_the_dead_letter_queue_once_its_receives_are_used_up() {
    let responses = session(&[
        r#"{"Command": "CreateQueue", "QueueName": "dlq"}"#,
        r#"{"Command": "CreateQueue", "QueueName": "jobs", "Attributes": {"DeadLetterQueue": "dlq", "MaxReceiveCount": 2}}"#,
        &send("jobs", "a"),
        &receive("jobs"),
        &tick(30),
        &receive("jobs"),
        &tick(30),
        &receive("jobs"),
        &counts("jobs"),
        &counts("dlq"),
        &tick(5),
        &receive("dlq"),
        &delete("jobs", "r-2"),
    ]);

    assert_eq!(responses[5]["Messages"][0]["ReceiveCount"], 2);
    assert_eq!(responses[7], json!({"ok": true, "Messages": []}));
    assert_eq!(
        responses[8]["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(
        responses[9]["Counts"],
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
    // The moved message keeps its id and sent time, and starts its receives afresh.
    // The move itself is not a delivery, so the next receipt is r-3.
    assert_eq!(
        responses[11]["Messages"],
        json!([{
            "MessageId": "m-1",
            "ReceiptHandle": "r-3",
            "Body": "a",
            "ReceiveCount": 1,
            "SentAt": 0,
            "FirstReceivedAt": 65,
        }])
    );
    assert_error(&responses[12], "ReceiptHandleExpired");
}

#[test]
fn moved_messages_do_not_count_toward_max_messages_and_examination_goes_on() {
    let responses = session(&[
        r#"{"Command": "CreateQueue", "QueueName": "dlq"}"#,
        r#"{"Command": "CreateQueue", "QueueName": "jobs", "Attributes": {"DeadLetterQueue": "dlq", "MaxReceiveCount": 1}}"#,
        &send("jobs", "a"),
        &send("jobs", "b"),
        &receive_one("jobs"),
        &tick(30),
        &receive_one("jobs"),
        &receive("dlq"),
    ]);

    assert_eq!(bodies(&responses[6]), ["b"]);
    assert_eq!(responses[6]["Messages"][0]["ReceiptHandle"], "r-2");
    assert_eq!(bodies(&responses[7]), ["a"]);
}

#[test]
fn fifo_move_frees_the_group_and_gives_sequence_numbers_on_the_dead_letter_queue() {
    let responses = session(&[
        r#"{"Command": "CreateQueue", "QueueName": "dlq.fifo", "Attributes": {"Fifo": true, "ContentBasedDeduplication": true}}"#,
        r#"{"Command": "CreateQueue", "QueueName": "src.fifo", "Attributes": {"Fifo": true, "ContentBasedDeduplication": true, "DeadLetterQueue": "dlq.fifo", "MaxReceiveCount": 1}}"#,
        &send_fifo("src.fifo", "a1", "g"),
        &send_fifo("src.fifo", "a2", "g"),
        &send_fifo("src.fifo", "b1", "h"),
        &receive_all("src.fifo"),
        &tick(30),
        &receive_all("src.fifo"),
        &receive_all("dlq.fifo"),
    ]);

    assert_eq!(bodies(&responses[5]), ["a1", "b1"]);
    // At t=30, a1 and b1 have used their one receive and move out. a2 was held
    // back only by a1, which is no longer in the queue, so it is delivered.
    assert_eq!(
        responses[7]["Messages"],
        json!([{
            "MessageId": "m-2",
            "ReceiptHandle": "r-3",
            "Body": "a2",
            "ReceiveCount": 1,
            "SentAt": 0,
            "FirstReceivedAt": 30,
            "GroupId": "g",
            "SequenceNumber": 2,
        }])
    );
    // Moved in the order they were examined, so they take sequence numbers 1 and 2.
    assert_eq!(
        responses[8]["Messages"],
        json!([
            {"MessageId": "m-1", "ReceiptHandle": "r-4", "Body": "a1", "ReceiveCount": 1,
             "SentAt": 0, "FirstReceivedAt": 30, "GroupId": "g", "SequenceNumber": 1},
            {"MessageId": "m-3", "ReceiptHandle": "r-5", "Body": "b1", "ReceiveCount": 1,
             "SentAt": 0, "FirstReceivedAt": 30, "GroupId": "h", "SequenceNumber": 2},
        ])
    );
}

#[test]
fn a_message_already_past_the_dead_letter_retention_is_dropped_on_arrival() {
    let responses = session(&[
        r#"{"Command": "CreateQueue", "QueueName": "dlq", "Attributes": {"RetentionSeconds": 60}}"#,
        r#"{"Command": "CreateQueue", "QueueName": "jobs", "Attributes": {"DeadLetterQueue": "dlq", "MaxReceiveCount": 1}}"#,
        &send("jobs", "a"),
        &receive("jobs"),
        &tick(100),
        &receive("jobs"),
        &counts("dlq"),
    ]);

    assert_eq!(responses[5], json!({"ok": true, "Messages": []}));
    assert_eq!(
        responses[6]["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn a_dead_letter_expires_by_its_own_retention_counted_from_when_it_was_sent() {
    let responses = session(&[
        r#"{"Command": "CreateQueue", "QueueName": "dlq", "Attributes": {"RetentionSeconds": 60}}"#,
        r#"{"Command": "CreateQueue", "QueueName": "jobs", "Attributes": {"DeadLetterQueue": "dlq", "MaxReceiveCount": 1}}"#,
        &send("jobs", "a"),
        &receive("jobs"),
        &tick(30),
        &receive("jobs"),
        &counts("dlq"),
        &tick(30),
        &counts("dlq"),
    ]);

    assert_eq!(responses[6]["Counts"]["Visible"], 1);
    assert_eq!(responses[8]["Counts"]["Visible"], 0);
}

#[test]
fn deleting_the_dead_letter_queue_stops_moves_and_the_message_is_delivered_again() {
    let responses = session(&[
        r#"{"Command": "CreateQueue", "QueueName": "dlq"}"#,
        r#"{"Command": "CreateQueue", "QueueName": "jobs", "Attributes": {"DeadLetterQueue": "dlq", "MaxReceiveCount": 1}}"#,
        &send("jobs", "a"),
        &receive("jobs"),
        r#"{"Command": "DeleteQueue", "QueueName": "dlq"}"#,
        &tick(30),
        &receive("jobs"),
    ]);

    assert_eq!(responses[6]["Messages"][0]["Body"], "a");
    assert_eq!(responses[6]["Messages"][0]["ReceiveCount"], 2);
    assert_eq!(responses[6]["Messages"][0]["ReceiptHandle"], "r-2");
}
