//! ReceiveMessage (section 6): delivery order, visibility, FIFO group rules
//! and dead-letter moves.

mod common;

use common::{fail, ok};
use miniq::Broker;
use serde_json::Value;

fn send(broker: &mut Broker, queue: &str, body: &str) {
    ok(
        broker,
        &format!(r#"{{"Command": "SendMessage", "QueueName": "{queue}", "Body": "{body}"}}"#),
    );
}

fn send_fifo(broker: &mut Broker, queue: &str, body: &str, group: &str) {
    ok(
        broker,
        &format!(
            r#"{{"Command": "SendMessage", "QueueName": "{queue}", "Body": "{body}", "GroupId": "{group}", "DeduplicationId": "{body}"}}"#
        ),
    );
}

/// One ReceiveMessage call with the default parameters.
fn receive(broker: &mut Broker, queue: &str) -> Vec<Value> {
    let line = format!(r#"{{"Command": "ReceiveMessage", "QueueName": "{queue}"}}"#);
    ok(broker, &line)["Messages"].as_array().unwrap().clone()
}

fn bodies(messages: &[Value]) -> Vec<&str> {
    messages
        .iter()
        .map(|m| m["Body"].as_str().unwrap())
        .collect()
}

fn counts(broker: &mut Broker, queue: &str) -> Value {
    ok(
        broker,
        &format!(r#"{{"Command": "GetQueueAttributes", "QueueName": "{queue}"}}"#),
    )["Counts"]
        .clone()
}

#[test]
fn an_empty_queue_returns_no_messages() {
    let mut broker = Broker::new();
    ok(
        &mut broker,
        r#"{"Command": "CreateQueue", "QueueName": "a"}"#,
    );
    assert!(receive(&mut broker, "a").is_empty());
}

#[test]
fn receive_returns_one_message_by_default_and_up_to_ten_in_arrival_order() {
    let mut broker = Broker::new();
    ok(
        &mut broker,
        r#"{"Command": "CreateQueue", "QueueName": "a"}"#,
    );
    for body in ["x", "y", "z"] {
        send(&mut broker, "a", body);
    }
    let first = ok(
        &mut broker,
        r#"{"Command": "ReceiveMessage", "QueueName": "a"}"#,
    );
    assert_eq!(bodies(first["Messages"].as_array().unwrap()), ["x"]);
    let rest = ok(
        &mut broker,
        r#"{"Command": "ReceiveMessage", "QueueName": "a", "MaxMessages": 10}"#,
    );
    assert_eq!(bodies(rest["Messages"].as_array().unwrap()), ["y", "z"]);
}

#[test]
fn a_delivery_carries_the_documented_members() {
    let mut broker = Broker::new();
    ok(
        &mut broker,
        r#"{"Command": "CreateQueue", "QueueName": "a"}"#,
    );
    ok(
        &mut broker,
        r#"{"Command": "SendMessage", "QueueName": "a", "Body": "hello"}"#,
    );
    ok(&mut broker, r#"{"Command": "Tick", "Seconds": 4}"#);
    let response = ok(
        &mut broker,
        r#"{"Command": "ReceiveMessage", "QueueName": "a"}"#,
    );
    let message = &response["Messages"][0];
    assert_eq!(message["MessageId"], "m-1");
    assert_eq!(message["ReceiptHandle"], "r-1");
    assert_eq!(message["Body"], "hello");
    assert_eq!(message["ReceiveCount"], 1);
    assert_eq!(message["SentAt"], 0);
    assert_eq!(message["FirstReceivedAt"], 4);
    assert!(message.get("GroupId").is_none());
    assert!(message.get("SequenceNumber").is_none());
}

#[test]
fn receipt_handles_are_numbered_in_delivery_order_over_all_queues() {
    let mut broker = Broker::new();
    ok(
        &mut broker,
        r#"{"Command": "CreateQueue", "QueueName": "a"}"#,
    );
    ok(
        &mut broker,
        r#"{"Command": "CreateQueue", "QueueName": "b"}"#,
    );
    send(&mut broker, "a", "1");
    send(&mut broker, "b", "2");
    send(&mut broker, "a", "3");
    let from_a = ok(
        &mut broker,
        r#"{"Command": "ReceiveMessage", "QueueName": "a", "MaxMessages": 2}"#,
    );
    let from_b = ok(
        &mut broker,
        r#"{"Command": "ReceiveMessage", "QueueName": "b"}"#,
    );
    assert_eq!(from_a["Messages"][0]["ReceiptHandle"], "r-1");
    assert_eq!(from_a["Messages"][1]["ReceiptHandle"], "r-2");
    assert_eq!(from_b["Messages"][0]["ReceiptHandle"], "r-3");
}

#[test]
fn a_delivered_message_is_in_flight_until_its_visibility_timeout_ends() {
    let mut broker = Broker::new();
    ok(
        &mut broker,
        r#"{"Command": "CreateQueue", "QueueName": "a", "Attributes": {"VisibilityTimeout": 10}}"#,
    );
    send(&mut broker, "a", "x");
    receive(&mut broker, "a");
    assert_eq!(
        counts(&mut broker, "a"),
        serde_json::json!({"Visible": 0, "InFlight": 1, "Delayed": 0})
    );
    assert!(receive(&mut broker, "a").is_empty());

    ok(&mut broker, r#"{"Command": "Tick", "Seconds": 10}"#);
    let again = receive(&mut broker, "a");
    assert_eq!(bodies(&again), ["x"]);
    assert_eq!(again[0]["ReceiveCount"], 2);
    assert_eq!(again[0]["ReceiptHandle"], "r-2");
}

#[test]
fn a_per_call_visibility_timeout_overrides_the_queue_default() {
    let mut broker = Broker::new();
    ok(
        &mut broker,
        r#"{"Command": "CreateQueue", "QueueName": "a"}"#,
    );
    send(&mut broker, "a", "x");
    ok(
        &mut broker,
        r#"{"Command": "ReceiveMessage", "QueueName": "a", "VisibilityTimeout": 0}"#,
    );
    assert_eq!(
        counts(&mut broker, "a")["Visible"],
        1,
        "zero timeout makes it visible at once"
    );
}

#[test]
fn first_received_at_is_kept_from_the_first_delivery() {
    let mut broker = Broker::new();
    ok(
        &mut broker,
        r#"{"Command": "CreateQueue", "QueueName": "a", "Attributes": {"VisibilityTimeout": 0}}"#,
    );
    send(&mut broker, "a", "x");
    ok(&mut broker, r#"{"Command": "Tick", "Seconds": 3}"#);
    receive(&mut broker, "a");
    ok(&mut broker, r#"{"Command": "Tick", "Seconds": 5}"#);
    let again = receive(&mut broker, "a");
    assert_eq!(again[0]["FirstReceivedAt"], 3);
    assert_eq!(again[0]["ReceiveCount"], 2);
}

#[test]
fn delayed_messages_are_not_delivered_before_their_time() {
    let mut broker = Broker::new();
    ok(
        &mut broker,
        r#"{"Command": "CreateQueue", "QueueName": "a", "Attributes": {"DelaySeconds": 5}}"#,
    );
    send(&mut broker, "a", "x");
    assert!(receive(&mut broker, "a").is_empty());
    ok(&mut broker, r#"{"Command": "Tick", "Seconds": 5}"#);
    assert_eq!(bodies(&receive(&mut broker, "a")), ["x"]);
}

#[test]
fn receive_checks_its_parameters() {
    let mut broker = Broker::new();
    ok(
        &mut broker,
        r#"{"Command": "CreateQueue", "QueueName": "a"}"#,
    );
    fail(
        &mut broker,
        r#"{"Command": "ReceiveMessage"}"#,
        "MissingParameter",
    );
    fail(
        &mut broker,
        r#"{"Command": "ReceiveMessage", "QueueName": "nope"}"#,
        "QueueDoesNotExist",
    );
    fail(
        &mut broker,
        r#"{"Command": "ReceiveMessage", "QueueName": "a", "MaxMessages": 0}"#,
        "InvalidParameterValue",
    );
    fail(
        &mut broker,
        r#"{"Command": "ReceiveMessage", "QueueName": "a", "MaxMessages": 11}"#,
        "InvalidParameterValue",
    );
    fail(
        &mut broker,
        r#"{"Command": "ReceiveMessage", "QueueName": "a", "VisibilityTimeout": 43201}"#,
        "InvalidParameterValue",
    );
}

#[test]
fn fifo_delivers_at_most_one_message_per_group_per_call() {
    let mut broker = Broker::new();
    ok(
        &mut broker,
        r#"{"Command": "CreateQueue", "QueueName": "f.fifo", "Attributes": {"Fifo": true, "VisibilityTimeout": 0}}"#,
    );
    send_fifo(&mut broker, "f.fifo", "a1", "g1");
    send_fifo(&mut broker, "f.fifo", "b1", "g2");
    send_fifo(&mut broker, "f.fifo", "a2", "g1");
    let response = ok(
        &mut broker,
        r#"{"Command": "ReceiveMessage", "QueueName": "f.fifo", "MaxMessages": 10}"#,
    );
    let messages = response["Messages"].as_array().unwrap();
    assert_eq!(bodies(messages), ["a1", "b1"]);
    assert_eq!(messages[0]["GroupId"], "g1");
    assert_eq!(messages[0]["SequenceNumber"], 1);
    assert_eq!(messages[1]["SequenceNumber"], 2);
}

#[test]
fn a_fifo_group_waits_while_its_earlier_message_is_in_flight() {
    let mut broker = Broker::new();
    ok(
        &mut broker,
        r#"{"Command": "CreateQueue", "QueueName": "f.fifo", "Attributes": {"Fifo": true}}"#,
    );
    send_fifo(&mut broker, "f.fifo", "a1", "g1");
    send_fifo(&mut broker, "f.fifo", "a2", "g1");
    assert_eq!(bodies(&receive(&mut broker, "f.fifo")), ["a1"]);
    assert!(
        receive(&mut broker, "f.fifo").is_empty(),
        "a2 must wait behind a1"
    );
}

#[test]
fn a_fifo_group_waits_while_its_earlier_message_is_delayed() {
    let mut broker = Broker::new();
    ok(
        &mut broker,
        r#"{"Command": "CreateQueue", "QueueName": "f.fifo", "Attributes": {"Fifo": true}}"#,
    );
    send_fifo(&mut broker, "f.fifo", "a1", "g1");
    ok(
        &mut broker,
        r#"{"Command": "SetQueueAttributes", "QueueName": "f.fifo", "Attributes": {"VisibilityTimeout": 0}}"#,
    );
    assert_eq!(bodies(&receive(&mut broker, "f.fifo")), ["a1"]);
    assert_eq!(bodies(&receive(&mut broker, "f.fifo")), ["a1"]);
}

#[test]
fn a_message_moves_to_the_dead_letter_queue_once_its_receive_count_reaches_the_limit() {
    let mut broker = Broker::new();
    ok(
        &mut broker,
        r#"{"Command": "CreateQueue", "QueueName": "dead"}"#,
    );
    ok(
        &mut broker,
        r#"{"Command": "CreateQueue", "QueueName": "jobs", "Attributes": {"VisibilityTimeout": 0, "DeadLetterQueue": "dead", "MaxReceiveCount": 2}}"#,
    );
    send(&mut broker, "jobs", "x");
    assert_eq!(bodies(&receive(&mut broker, "jobs")), ["x"]);
    assert_eq!(bodies(&receive(&mut broker, "jobs")), ["x"]);
    assert!(receive(&mut broker, "jobs").is_empty());
    assert_eq!(counts(&mut broker, "jobs")["Visible"], 0);

    let moved = receive(&mut broker, "dead");
    assert_eq!(bodies(&moved), ["x"]);
    assert_eq!(
        moved[0]["MessageId"], "m-1",
        "a dead-lettered message keeps its id"
    );
    assert_eq!(
        moved[0]["ReceiveCount"], 1,
        "its receive count starts again at zero"
    );
    assert_eq!(moved[0]["SentAt"], 0);
}

#[test]
fn dead_letter_moves_do_not_count_toward_max_messages() {
    let mut broker = Broker::new();
    ok(
        &mut broker,
        r#"{"Command": "CreateQueue", "QueueName": "dead"}"#,
    );
    ok(
        &mut broker,
        r#"{"Command": "CreateQueue", "QueueName": "jobs", "Attributes": {"VisibilityTimeout": 0, "DeadLetterQueue": "dead", "MaxReceiveCount": 1}}"#,
    );
    send(&mut broker, "jobs", "gone");
    receive(&mut broker, "jobs");
    send(&mut broker, "jobs", "a");
    send(&mut broker, "jobs", "b");
    let response = ok(
        &mut broker,
        r#"{"Command": "ReceiveMessage", "QueueName": "jobs", "MaxMessages": 2}"#,
    );
    assert_eq!(bodies(response["Messages"].as_array().unwrap()), ["a", "b"]);
    assert_eq!(counts(&mut broker, "dead")["Visible"], 1);
}

#[test]
fn a_fifo_dead_letter_takes_the_next_sequence_number_of_the_dead_letter_queue() {
    let mut broker = Broker::new();
    ok(
        &mut broker,
        r#"{"Command": "CreateQueue", "QueueName": "dead.fifo", "Attributes": {"Fifo": true}}"#,
    );
    ok(
        &mut broker,
        r#"{"Command": "CreateQueue", "QueueName": "jobs.fifo", "Attributes": {"Fifo": true, "VisibilityTimeout": 0, "DeadLetterQueue": "dead.fifo", "MaxReceiveCount": 1}}"#,
    );
    send_fifo(&mut broker, "jobs.fifo", "x", "g");
    ok(
        &mut broker,
        r#"{"Command": "ReceiveMessage", "QueueName": "jobs.fifo"}"#,
    );
    send_fifo(&mut broker, "jobs.fifo", "y", "g");
    ok(
        &mut broker,
        r#"{"Command": "ReceiveMessage", "QueueName": "jobs.fifo"}"#,
    );
    let moved = receive(&mut broker, "dead.fifo");
    assert_eq!(bodies(&moved), ["x"]);
    assert_eq!(moved[0]["SequenceNumber"], 1);
    assert_eq!(moved[0]["GroupId"], "g");
}

#[test]
fn a_dead_lettered_message_already_too_old_for_the_dead_letter_queue_is_gone() {
    let mut broker = Broker::new();
    ok(
        &mut broker,
        r#"{"Command": "CreateQueue", "QueueName": "dead", "Attributes": {"RetentionSeconds": 60}}"#,
    );
    ok(
        &mut broker,
        r#"{"Command": "CreateQueue", "QueueName": "jobs", "Attributes": {"DeadLetterQueue": "dead", "MaxReceiveCount": 1}}"#,
    );
    send(&mut broker, "jobs", "x");
    receive(&mut broker, "jobs");
    ok(&mut broker, r#"{"Command": "Tick", "Seconds": 70}"#);
    assert!(receive(&mut broker, "jobs").is_empty());
    assert_eq!(counts(&mut broker, "dead")["Visible"], 0);
}

#[test]
fn a_dead_letter_queue_that_is_deleted_stops_the_move() {
    let mut broker = Broker::new();
    ok(
        &mut broker,
        r#"{"Command": "CreateQueue", "QueueName": "dead"}"#,
    );
    ok(
        &mut broker,
        r#"{"Command": "CreateQueue", "QueueName": "jobs", "Attributes": {"DeadLetterQueue": "dead", "MaxReceiveCount": 1}}"#,
    );
    ok(
        &mut broker,
        r#"{"Command": "DeleteQueue", "QueueName": "dead"}"#,
    );
    send(&mut broker, "jobs", "x");
    receive(&mut broker, "jobs");
    ok(&mut broker, r#"{"Command": "Tick", "Seconds": 30}"#);
    assert_eq!(
        bodies(&receive(&mut broker, "jobs")),
        ["x"],
        "no dead-letter setting, so it is delivered again"
    );
}
