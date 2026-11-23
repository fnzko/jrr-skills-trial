mod common;

use common::{error_code, run};
use serde_json::{Value, json};

fn create_fifo(name: &str, extra: Value) -> Value {
    let mut attributes = json!({"Fifo": true});
    for (key, value) in extra.as_object().expect("extra attributes are an object") {
        attributes[key] = value.clone();
    }
    json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attributes})
}

fn send(queue: &str, body: &str, group: &str, dedup: &str) -> Value {
    json!({"Command": "SendMessage", "QueueName": queue, "Body": body, "GroupId": group, "DeduplicationId": dedup})
}

fn receive(queue: &str, max: u32) -> Value {
    json!({"Command": "ReceiveMessage", "QueueName": queue, "MaxMessages": max})
}

fn tick(seconds: u32) -> Value {
    json!({"Command": "Tick", "Seconds": seconds})
}

fn bodies(response: &Value) -> Vec<&str> {
    response["Messages"]
        .as_array()
        .expect("a receive result has Messages")
        .iter()
        .map(|m| m["Body"].as_str().expect("body is a string"))
        .collect()
}

#[test]
fn fifo_send_rules_in_order() {
    let out = run(&[
        create_fifo("f.fifo", json!({})),
        json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "b", "DelaySeconds": 1}),
        json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "b"}),
        json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "b", "GroupId": "g"}),
        send("f.fifo", "b", "g", "d"),
    ]);
    assert_eq!(error_code(&out[1]), "UnsupportedOperation");
    assert_eq!(error_code(&out[2]), "MissingParameter");
    assert_eq!(error_code(&out[3]), "MissingParameter");
    assert_eq!(
        out[4],
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
}

#[test]
fn content_based_deduplication_uses_the_body() {
    let out = run(&[
        create_fifo("f.fifo", json!({"ContentBasedDeduplication": true})),
        json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "same", "GroupId": "g"}),
        json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "same", "GroupId": "other"}),
        json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "diff", "GroupId": "g"}),
    ]);
    assert_eq!(out[2], out[1]);
    assert_eq!(out[3]["MessageId"], "m-2");
    assert_eq!(out[3]["SequenceNumber"], 2);
}

#[test]
fn duplicate_window_is_300_seconds_and_not_extended_by_duplicates() {
    let out = run(&[
        create_fifo("f.fifo", json!({})),
        send("f.fifo", "a", "g", "d"),
        tick(299),
        send("f.fifo", "a", "g", "d"),
        tick(1),
        send("f.fifo", "a", "g", "d"),
        tick(299),
        send("f.fifo", "a", "g", "d"),
        tick(1),
        send("f.fifo", "a", "g", "d"),
    ]);
    assert_eq!(out[3]["MessageId"], "m-1");
    assert_eq!(out[5]["MessageId"], "m-2");
    assert_eq!(out[7]["MessageId"], "m-2");
    assert_eq!(out[9]["MessageId"], "m-3");
}

#[test]
fn deduplication_outlives_the_message() {
    let out = run(&[
        create_fifo("f.fifo", json!({})),
        send("f.fifo", "a", "g", "d"),
        json!({"Command": "ReceiveMessage", "QueueName": "f.fifo"}),
        json!({"Command": "DeleteMessage", "QueueName": "f.fifo", "ReceiptHandle": "r-1"}),
        send("f.fifo", "a", "g", "d"),
    ]);
    assert_eq!(
        out[4],
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
}

#[test]
fn fifo_delivers_one_message_per_group_and_blocks_while_earlier_is_in_flight() {
    let out = run(&[
        create_fifo("f.fifo", json!({"VisibilityTimeout": 10})),
        send("f.fifo", "a1", "a", "1"),
        send("f.fifo", "a2", "a", "2"),
        send("f.fifo", "b1", "b", "3"),
        receive("f.fifo", 10),
        receive("f.fifo", 10),
        tick(10),
        receive("f.fifo", 10),
    ]);
    assert_eq!(bodies(&out[4]), ["a1", "b1"]);
    assert_eq!(bodies(&out[5]), Vec::<&str>::new());
    assert_eq!(bodies(&out[7]), ["a1", "b1"]);
}

#[test]
fn deleting_the_earliest_unblocks_the_group() {
    let out = run(&[
        create_fifo("f.fifo", json!({})),
        send("f.fifo", "a1", "a", "1"),
        send("f.fifo", "a2", "a", "2"),
        receive("f.fifo", 1),
        json!({"Command": "DeleteMessage", "QueueName": "f.fifo", "ReceiptHandle": "r-1"}),
        receive("f.fifo", 1),
    ]);
    assert_eq!(bodies(&out[5]), ["a2"]);
}

fn dead_letter_setup(dlq_attributes: Value) -> Vec<Value> {
    vec![
        json!({"Command": "CreateQueue", "QueueName": "dead", "Attributes": dlq_attributes}),
        json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": {
            "VisibilityTimeout": 0, "DeadLetterQueue": "dead", "MaxReceiveCount": 2}}),
    ]
}

#[test]
fn message_moves_to_dead_letter_queue_after_max_receives() {
    let mut commands = dead_letter_setup(json!({}));
    commands.extend([
        json!({"Command": "SendMessage", "QueueName": "q", "Body": "poison"}),
        json!({"Command": "SendMessage", "QueueName": "q", "Body": "later"}),
        tick(3),
        receive("q", 1),
        receive("q", 1),
        receive("q", 1),
        receive("dead", 1),
        json!({"Command": "GetQueueAttributes", "QueueName": "q"}),
    ]);
    let out = run(&commands);
    assert_eq!(bodies(&out[5]), ["poison"]);
    assert_eq!(bodies(&out[6]), ["poison"]);
    // The third receive moves "poison" without counting it, then delivers "later".
    assert_eq!(bodies(&out[7]), ["later"]);
    let dead = &out[8]["Messages"][0];
    assert_eq!(dead["Body"], "poison");
    assert_eq!(dead["MessageId"], "m-1");
    assert_eq!(dead["ReceiveCount"], 1);
    assert_eq!(dead["SentAt"], 0);
    assert_eq!(dead["FirstReceivedAt"], 3);
    assert_eq!(out[9]["Counts"]["Visible"], 1);
}

#[test]
fn dead_letter_arrival_is_subject_to_the_target_retention() {
    let mut commands = dead_letter_setup(json!({"RetentionSeconds": 60}));
    commands.extend([
        json!({"Command": "SendMessage", "QueueName": "q", "Body": "old"}),
        receive("q", 1),
        receive("q", 1),
        tick(60),
        receive("q", 1),
        json!({"Command": "GetQueueAttributes", "QueueName": "dead"}),
    ]);
    let out = run(&commands);
    assert_eq!(bodies(&out[6]), Vec::<&str>::new());
    assert_eq!(
        out[7]["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn fifo_dead_letter_move_takes_a_new_sequence_number_and_keeps_the_group() {
    let out = run(&[
        create_fifo("dead.fifo", json!({})),
        create_fifo(
            "q.fifo",
            json!({"VisibilityTimeout": 0, "DeadLetterQueue": "dead.fifo", "MaxReceiveCount": 1}),
        ),
        send("dead.fifo", "existing", "x", "e"),
        send("q.fifo", "poison", "g", "p"),
        receive("q.fifo", 1),
        receive("q.fifo", 1),
        receive("dead.fifo", 10),
    ]);
    let messages = out[6]["Messages"].as_array().expect("Messages array");
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[1]["Body"], "poison");
    assert_eq!(messages[1]["GroupId"], "g");
    assert_eq!(messages[1]["SequenceNumber"], 2);
}

#[test]
fn filling_a_queue_to_the_limit_is_fast_and_the_next_send_fails() {
    let mut commands = vec![json!({"Command": "CreateQueue", "QueueName": "big"})];
    commands.extend(
        (0..100_001).map(|_| json!({"Command": "SendMessage", "QueueName": "big", "Body": "x"})),
    );
    commands.push(json!({"Command": "GetQueueAttributes", "QueueName": "big"}));
    let out = run(&commands);
    assert_eq!(out[100_000]["MessageId"], "m-100000");
    assert_eq!(error_code(&out[100_001]), "LimitExceeded");
    assert_eq!(out[100_002]["Counts"]["Visible"], 100_000);
}
