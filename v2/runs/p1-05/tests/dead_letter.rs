mod common;

use common::*;
use serde_json::json;

fn dead_letter_pair(max_receive_count: u64) -> Vec<serde_json::Value> {
    vec![
        create("dlq", json!({})),
        create(
            "q",
            json!({"VisibilityTimeout": 0, "DeadLetterQueue": "dlq", "MaxReceiveCount": max_receive_count}),
        ),
    ]
}

#[test]
fn message_moves_to_the_dead_letter_queue_once_received_max_receive_count_times() {
    let mut commands = dead_letter_pair(2);
    commands.extend([
        send("q", "a"),
        receive("q"),
        receive("q"),
        receive("q"),
        counts("q"),
        counts("dlq"),
        receive("dlq"),
    ]);
    let responses = run_values(&commands);
    assert_eq!(responses[3]["Messages"][0]["ReceiveCount"], 1);
    assert_eq!(responses[4]["Messages"][0]["ReceiveCount"], 2);
    assert_eq!(responses[5], json!({"ok": true, "Messages": []}));
    assert_eq!(state_counts(&responses[6]), [0, 0, 0]);
    assert_eq!(state_counts(&responses[7]), [1, 0, 0]);
    // Same MessageId and body, receive count and first-receive time start over.
    assert_eq!(
        responses[8]["Messages"][0],
        json!({"MessageId": "m-1", "ReceiptHandle": "r-3", "Body": "a",
               "ReceiveCount": 1, "SentAt": 0, "FirstReceivedAt": 0})
    );
}

#[test]
fn moved_message_keeps_its_send_time_and_forgets_the_first_receive_time() {
    let mut commands = dead_letter_pair(1);
    commands.extend([
        send("q", "a"),
        receive("q"),
        tick(7),
        receive("q"),
        tick(5),
        receive("dlq"),
    ]);
    let responses = run_values(&commands);
    let moved = &responses[7]["Messages"][0];
    assert_eq!(moved["SentAt"], 0);
    assert_eq!(moved["FirstReceivedAt"], 12);
    assert_eq!(moved["ReceiveCount"], 1);
}

#[test]
fn moved_messages_do_not_count_toward_max_messages_and_the_pass_goes_on() {
    let mut commands = dead_letter_pair(1);
    commands.extend([
        send("q", "old1"),
        send("q", "old2"),
        receive_many("q", 2),
        send("q", "new1"),
        send("q", "new2"),
        send("q", "new3"),
        // old1 and old2 are exhausted and move; two of the new ones are delivered.
        receive_many("q", 2),
        counts("q"),
        receive_many("dlq", 10),
    ]);
    let responses = run_values(&commands);
    assert_eq!(bodies(&responses[8]), ["new1", "new2"]);
    // The queue has a visibility timeout of 0, so the delivered messages are visible again.
    assert_eq!(state_counts(&responses[9]), [3, 0, 0]);
    assert_eq!(bodies(&responses[10]), ["old1", "old2"]);
}

#[test]
fn only_visible_messages_are_examined_for_the_dead_letter_move() {
    let commands = vec![
        create("dlq", json!({})),
        create(
            "q",
            json!({"VisibilityTimeout": 100, "DeadLetterQueue": "dlq", "MaxReceiveCount": 1}),
        ),
        send("q", "a"),
        receive("q"),
        receive("q"),
        counts("dlq"),
        tick(100),
        receive("q"),
        counts("dlq"),
    ];
    let responses = run_values(&commands);
    assert_eq!(state_counts(&responses[5]), [0, 0, 0]);
    assert_eq!(state_counts(&responses[8]), [1, 0, 0]);
}

#[test]
fn the_dead_letter_queue_ignores_its_message_limit_and_deduplication() {
    // dlq is a FIFO queue; the move records no deduplication id, so the same id can be sent after it.
    let responses = run_values(&[
        create("dlq.fifo", json!({"Fifo": true})),
        create(
            "q.fifo",
            json!({"Fifo": true, "DeadLetterQueue": "dlq.fifo", "MaxReceiveCount": 1}),
        ),
        json!({"Command": "SendMessage", "QueueName": "q.fifo", "Body": "a", "GroupId": "g", "DeduplicationId": "k"}),
        json!({"Command": "SetQueueAttributes", "QueueName": "q.fifo", "Attributes": {"VisibilityTimeout": 0}}),
        receive("q.fifo"),
        receive("q.fifo"),
        json!({"Command": "SendMessage", "QueueName": "dlq.fifo", "Body": "b", "GroupId": "g", "DeduplicationId": "k"}),
        receive_many("dlq.fifo", 10),
    ]);
    // The moved message took sequence number 1 of dlq.fifo, so the send is number 2.
    assert_eq!(
        responses[6],
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2})
    );
    let messages = responses[7]["Messages"].as_array().unwrap();
    assert_eq!(messages[0]["MessageId"], "m-1");
    assert_eq!(messages[0]["GroupId"], "g");
    assert_eq!(messages[0]["SequenceNumber"], 1);
    // Group "g" has the moved message first, so only it is delivered.
    assert_eq!(messages.len(), 1);
}

#[test]
fn a_message_too_old_for_the_dead_letter_queue_is_removed_on_arrival() {
    let responses = run_values(&[
        create("dlq", json!({"RetentionSeconds": 60})),
        create(
            "q",
            json!({"VisibilityTimeout": 0, "DeadLetterQueue": "dlq", "MaxReceiveCount": 1}),
        ),
        send("q", "a"),
        receive("q"),
        tick(60),
        receive("q"),
        counts("dlq"),
    ]);
    assert_eq!(state_counts(&responses[6]), [0, 0, 0]);
}

#[test]
fn a_message_in_a_fifo_queue_moves_in_group_order() {
    let send_to = |body: &str| json!({"Command": "SendMessage", "QueueName": "q.fifo", "Body": body, "GroupId": "g", "DeduplicationId": body});
    let responses = run_values(&[
        create("dlq.fifo", json!({"Fifo": true})),
        create(
            "q.fifo",
            json!({"Fifo": true, "VisibilityTimeout": 0, "DeadLetterQueue": "dlq.fifo", "MaxReceiveCount": 1}),
        ),
        send_to("one"),
        send_to("two"),
        receive("q.fifo"),
        // "one" moves, which frees "two" within the same call.
        receive("q.fifo"),
        receive_many("dlq.fifo", 10),
    ]);
    assert_eq!(bodies(&responses[4]), ["one"]);
    assert_eq!(bodies(&responses[5]), ["two"]);
    let moved = responses[6]["Messages"].as_array().unwrap();
    assert_eq!(moved[0]["Body"], "one");
    assert_eq!(moved[0]["SequenceNumber"], 1);
}

#[test]
fn a_new_dead_letter_configuration_applies_from_the_next_receive() {
    let responses = run_values(&[
        create("dlq", json!({})),
        create("q", json!({"VisibilityTimeout": 0})),
        send("q", "a"),
        receive("q"),
        receive("q"),
        json!({"Command": "SetQueueAttributes", "QueueName": "q",
               "Attributes": {"DeadLetterQueue": "dlq", "MaxReceiveCount": 2}}),
        counts("q"),
        counts("dlq"),
        receive("q"),
        counts("dlq"),
    ]);
    assert_eq!(state_counts(&responses[6]), [1, 0, 0]);
    assert_eq!(state_counts(&responses[7]), [0, 0, 0]);
    assert_eq!(responses[8], json!({"ok": true, "Messages": []}));
    assert_eq!(state_counts(&responses[9]), [1, 0, 0]);
}

#[test]
fn deleting_the_dead_letter_queue_turns_the_dead_lettering_off() {
    let mut commands = dead_letter_pair(1);
    commands.extend([
        send("q", "a"),
        receive("q"),
        json!({"Command": "DeleteQueue", "QueueName": "dlq"}),
        receive("q"),
    ]);
    let responses = run_values(&commands);
    assert_eq!(bodies(&responses[5]), ["a"]);
    assert_eq!(responses[5]["Messages"][0]["ReceiveCount"], 2);
}
