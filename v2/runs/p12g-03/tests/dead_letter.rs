mod common;

use common::{error_code, exec};
use serde_json::{Value, json};

fn create(name: &str, attributes: Value) -> Value {
    json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attributes})
}

fn with_dead_letter(visibility: u32, max_receive_count: u32) -> Value {
    json!({"VisibilityTimeout": visibility, "DeadLetterQueue": "d", "MaxReceiveCount": max_receive_count})
}

fn send(queue: &str, body: &str) -> Value {
    json!({"Command": "SendMessage", "QueueName": queue, "Body": body})
}

fn receive(queue: &str) -> Value {
    json!({"Command": "ReceiveMessage", "QueueName": queue, "MaxMessages": 10})
}

fn counts(queue: &str) -> Value {
    json!({"Command": "GetQueueAttributes", "QueueName": queue})
}

fn tick(seconds: u32) -> Value {
    json!({"Command": "Tick", "Seconds": seconds})
}

#[test]
fn a_message_received_max_receive_count_times_moves_on_the_next_receive() {
    let responses = exec(&[
        create("d", json!({})),
        create("q", with_dead_letter(0, 2)),
        tick(4),
        send("q", "poison"),
        tick(6),
        receive("q"),
        receive("q"),
        receive("q"),
        counts("q"),
        receive("d"),
    ]);
    assert_eq!(responses[5]["Messages"][0]["ReceiveCount"], json!(1));
    assert_eq!(responses[6]["Messages"][0]["ReceiveCount"], json!(2));
    assert_eq!(responses[7], json!({"ok": true, "Messages": []}));
    assert_eq!(
        responses[8]["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(
        responses[9],
        json!({"ok": true, "Messages": [{
            "MessageId": "m-1", "ReceiptHandle": "r-3", "Body": "poison",
            "ReceiveCount": 1, "SentAt": 4, "FirstReceivedAt": 10
        }]})
    );
}

#[test]
fn a_message_in_flight_is_not_moved_until_it_is_visible_again() {
    let responses = exec(&[
        create("d", json!({})),
        create("q", with_dead_letter(30, 1)),
        send("q", "a"),
        receive("q"),
        counts("d"),
        tick(30),
        receive("q"),
        counts("d"),
    ]);
    assert_eq!(responses[4]["Counts"]["Visible"], json!(0));
    assert_eq!(responses[6], json!({"ok": true, "Messages": []}));
    assert_eq!(responses[7]["Counts"]["Visible"], json!(1));
}

#[test]
fn moved_messages_do_not_count_toward_max_messages_and_their_handles_expire() {
    let responses = exec(&[
        create("d", json!({})),
        create("q", with_dead_letter(0, 1)),
        send("q", "old"),
        receive("q"),
        send("q", "new"),
        json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 1}),
        json!({"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": "r-1"}),
        counts("d"),
    ]);
    assert_eq!(responses[5]["Messages"][0]["Body"], json!("new"));
    assert_eq!(error_code(&responses[6]), Some("ReceiptHandleExpired"));
    assert_eq!(responses[7]["Counts"]["Visible"], json!(1));
}

#[test]
fn a_dead_lettered_fifo_message_unblocks_its_group_and_takes_a_new_sequence_number() {
    let responses = exec(&[
        create("d.fifo", json!({"Fifo": true})),
        create(
            "q.fifo",
            json!({"Fifo": true, "VisibilityTimeout": 0, "DeadLetterQueue": "d.fifo", "MaxReceiveCount": 1}),
        ),
        json!({"Command": "SendMessage", "QueueName": "d.fifo", "Body": "x", "GroupId": "other", "DeduplicationId": "keep"}),
        json!({"Command": "SendMessage", "QueueName": "q.fifo", "Body": "a1", "GroupId": "A", "DeduplicationId": "dup"}),
        json!({"Command": "SendMessage", "QueueName": "q.fifo", "Body": "a2", "GroupId": "A", "DeduplicationId": "a2"}),
        receive("q.fifo"),
        receive("q.fifo"),
        receive("d.fifo"),
        json!({"Command": "SendMessage", "QueueName": "d.fifo", "Body": "y", "GroupId": "A", "DeduplicationId": "dup"}),
    ]);
    assert_eq!(responses[5]["Messages"][0]["Body"], json!("a1"));
    assert_eq!(responses[6]["Messages"].as_array().map(Vec::len), Some(1));
    assert_eq!(responses[6]["Messages"][0]["Body"], json!("a2"));
    let in_dead_letter_queue = &responses[7]["Messages"];
    assert_eq!(in_dead_letter_queue[1]["Body"], json!("a1"));
    assert_eq!(in_dead_letter_queue[1]["GroupId"], json!("A"));
    assert_eq!(in_dead_letter_queue[1]["SequenceNumber"], json!(2));
    assert_eq!(in_dead_letter_queue[1]["MessageId"], json!("m-2"));
    assert_eq!(
        responses[8],
        json!({"ok": true, "MessageId": "m-4", "SequenceNumber": 3}),
        "the move did not record its deduplication id"
    );
}

#[test]
fn retention_of_the_dead_letter_queue_applies_from_the_original_sent_time() {
    let responses = exec(&[
        create("d", json!({"RetentionSeconds": 60})),
        create("q", with_dead_letter(0, 1)),
        send("q", "old"),
        receive("q"),
        tick(100),
        receive("q"),
        counts("d"),
    ]);
    assert_eq!(responses[5], json!({"ok": true, "Messages": []}));
    assert_eq!(
        responses[6]["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn a_new_dead_letter_configuration_applies_from_the_next_receive() {
    let responses = exec(&[
        create("d", json!({})),
        create("e", json!({})),
        create("q", json!({"VisibilityTimeout": 0})),
        send("q", "a"),
        receive("q"),
        receive("q"),
        json!({"Command": "SetQueueAttributes", "QueueName": "q",
               "Attributes": {"DeadLetterQueue": "e", "MaxReceiveCount": 2}}),
        receive("q"),
        counts("e"),
        counts("d"),
    ]);
    assert_eq!(responses[7], json!({"ok": true, "Messages": []}));
    assert_eq!(responses[8]["Counts"]["Visible"], json!(1));
    assert_eq!(responses[9]["Counts"]["Visible"], json!(0));
}

#[test]
fn deleting_the_dead_letter_queue_stops_moves() {
    let responses = exec(&[
        create("d", json!({})),
        create("q", with_dead_letter(0, 1)),
        send("q", "a"),
        receive("q"),
        json!({"Command": "DeleteQueue", "QueueName": "d"}),
        receive("q"),
    ]);
    assert_eq!(responses[5]["Messages"][0]["ReceiveCount"], json!(2));
}
