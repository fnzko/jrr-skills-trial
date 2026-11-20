mod common;

use common::{error_code, ok, session};
use serde_json::{Value, json};

fn create(name: &str, attributes: Value) -> Value {
    json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attributes})
}

fn send(queue: &str, body: &str) -> Value {
    json!({"Command": "SendMessage", "QueueName": queue, "Body": body})
}

fn send_fifo(queue: &str, body: &str, group: &str, dedup: &str) -> Value {
    json!({"Command": "SendMessage", "QueueName": queue, "Body": body,
           "GroupId": group, "DeduplicationId": dedup})
}

/// Receives with a visibility timeout of 0, so the message is visible again at once.
fn receive(queue: &str, max: u32) -> Value {
    json!({"Command": "ReceiveMessage", "QueueName": queue, "MaxMessages": max,
           "VisibilityTimeout": 0})
}

fn tick(seconds: u64) -> Value {
    json!({"Command": "Tick", "Seconds": seconds})
}

fn counts(queue: &str) -> Value {
    json!({"Command": "GetQueueAttributes", "QueueName": queue})
}

fn bodies(response: &Value) -> Vec<&str> {
    response["Messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|message| message["Body"].as_str().unwrap())
        .collect()
}

#[test]
fn a_message_moves_after_max_receive_count_deliveries_and_starts_over() {
    let responses = session(&[
        create("dead", json!({})),
        create(
            "jobs",
            json!({"DeadLetterQueue": "dead", "MaxReceiveCount": 2}),
        ),
        tick(4),
        send("jobs", "x"),
        tick(1),
        receive("jobs", 1),
        receive("jobs", 1),
        tick(1),
        receive("jobs", 1),
        counts("jobs"),
        counts("dead"),
        json!({"Command": "DeleteMessage", "QueueName": "jobs", "ReceiptHandle": "r-2"}),
        tick(1),
        receive("dead", 1),
    ]);
    assert_eq!(responses[5]["Messages"][0]["ReceiveCount"], 1);
    assert_eq!(responses[6]["Messages"][0]["ReceiveCount"], 2);
    assert_eq!(responses[8], json!({"ok": true, "Messages": []}));
    assert_eq!(
        responses[9]["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(
        responses[10]["Counts"],
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(error_code(&responses[11]), "ReceiptHandleExpired");
    assert_eq!(
        responses[13],
        json!({"ok": true, "Messages": [{
            "MessageId": "m-1",
            "ReceiptHandle": "r-3",
            "Body": "x",
            "ReceiveCount": 1,
            "SentAt": 4,
            "FirstReceivedAt": 7,
        }]})
    );
}

#[test]
fn a_moved_message_does_not_count_toward_the_maximum() {
    let responses = session(&[
        create("dead", json!({})),
        create(
            "jobs",
            json!({"DeadLetterQueue": "dead", "MaxReceiveCount": 1}),
        ),
        send("jobs", "a"),
        send("jobs", "b"),
        receive("jobs", 1),
        receive("jobs", 1),
        receive("dead", 10),
    ]);
    assert_eq!(bodies(&responses[4]), ["a"]);
    assert_eq!(bodies(&responses[5]), ["b"]);
    assert_eq!(bodies(&responses[6]), ["a"]);
}

#[test]
fn the_examination_stops_once_the_maximum_is_delivered() {
    let responses = session(&[
        create("dead", json!({})),
        create(
            "jobs",
            json!({"DeadLetterQueue": "dead", "MaxReceiveCount": 1}),
        ),
        send("jobs", "a"),
        json!({"Command": "SendMessage", "QueueName": "jobs", "Body": "d", "DelaySeconds": 5}),
        send("jobs", "c"),
        receive("jobs", 10),
        tick(5),
        receive("jobs", 1),
        counts("dead"),
    ]);
    assert_eq!(bodies(&responses[5]), ["a", "c"]);
    // a is moved and d is delivered; c, which is also at the limit, is never examined.
    assert_eq!(bodies(&responses[7]), ["d"]);
    assert_eq!(responses[8]["Counts"]["Visible"], 1);
}

#[test]
fn a_fifo_move_frees_the_group_and_takes_the_next_sequence_number() {
    let responses = session(&[
        create("dead.fifo", json!({"Fifo": true})),
        create(
            "jobs.fifo",
            json!({"Fifo": true, "DeadLetterQueue": "dead.fifo", "MaxReceiveCount": 1}),
        ),
        send_fifo("dead.fifo", "direct", "z", "d"),
        send_fifo("jobs.fifo", "g1", "g", "1"),
        send_fifo("jobs.fifo", "g2", "g", "2"),
        receive("jobs.fifo", 10),
        receive("jobs.fifo", 10),
        receive("dead.fifo", 10),
    ]);
    assert_eq!(bodies(&responses[5]), ["g1"]);
    assert_eq!(bodies(&responses[6]), ["g2"]);
    assert_eq!(
        responses[7]["Messages"],
        json!([
            {"MessageId": "m-1", "ReceiptHandle": "r-3", "Body": "direct", "ReceiveCount": 1,
             "SentAt": 0, "FirstReceivedAt": 0, "GroupId": "z", "SequenceNumber": 1},
            {"MessageId": "m-2", "ReceiptHandle": "r-4", "Body": "g1", "ReceiveCount": 1,
             "SentAt": 0, "FirstReceivedAt": 0, "GroupId": "g", "SequenceNumber": 2},
        ])
    );
}

#[test]
fn a_message_too_old_for_the_dead_letter_queue_is_dropped_but_uses_a_sequence_number() {
    let responses = session(&[
        create("dead.fifo", json!({"Fifo": true, "RetentionSeconds": 60})),
        create(
            "jobs.fifo",
            json!({"Fifo": true, "DeadLetterQueue": "dead.fifo", "MaxReceiveCount": 1}),
        ),
        send_fifo("jobs.fifo", "old", "g", "1"),
        receive("jobs.fifo", 10),
        tick(70),
        send_fifo("jobs.fifo", "young", "h", "2"),
        receive("jobs.fifo", 10),
        receive("jobs.fifo", 10),
        receive("dead.fifo", 10),
    ]);
    assert_eq!(bodies(&responses[6]), ["young"]);
    assert_eq!(bodies(&responses[7]), Vec::<&str>::new());
    assert_eq!(responses[8]["Messages"][0]["Body"], "young");
    assert_eq!(responses[8]["Messages"][0]["SequenceNumber"], 2);
    assert_eq!(responses[8]["Messages"][0]["SentAt"], 70);
}

#[test]
fn a_move_neither_checks_nor_records_deduplication_ids() {
    let responses = session(&[
        create("dead.fifo", json!({"Fifo": true})),
        create(
            "jobs.fifo",
            json!({"Fifo": true, "DeadLetterQueue": "dead.fifo", "MaxReceiveCount": 1}),
        ),
        send_fifo("dead.fifo", "x", "g", "same"),
        send_fifo("jobs.fifo", "x", "g", "same"),
        receive("jobs.fifo", 1),
        receive("jobs.fifo", 1),
        counts("dead.fifo"),
        send_fifo("dead.fifo", "again", "g", "same"),
        send_fifo("dead.fifo", "other", "g", "moved-id"),
    ]);
    assert_eq!(responses[6]["Counts"]["Visible"], 2);
    assert_eq!(responses[7]["MessageId"], "m-1");
    assert_eq!(responses[8]["MessageId"], "m-3");
}

#[test]
fn a_move_ignores_the_message_limit_of_the_dead_letter_queue() {
    let mut commands = vec![
        create("dead", json!({})),
        create(
            "jobs",
            json!({"DeadLetterQueue": "dead", "MaxReceiveCount": 1}),
        ),
        send("jobs", "last"),
        receive("jobs", 1),
    ];
    commands.extend((0..100_000).map(|_| send("dead", "x")));
    commands.push(receive("jobs", 1));
    commands.push(counts("dead"));
    commands.push(send("dead", "full"));
    let responses = session(&commands);
    let last = responses.len() - 1;
    assert_eq!(responses[last - 1]["Counts"]["Visible"], 100_001);
    assert_eq!(error_code(&responses[last]), "LimitExceeded");
}

#[test]
fn a_replaced_dead_letter_configuration_applies_from_the_next_receive() {
    let responses = session(&[
        create("dead1", json!({})),
        create("dead2", json!({})),
        create(
            "jobs",
            json!({"DeadLetterQueue": "dead1", "MaxReceiveCount": 5}),
        ),
        send("jobs", "x"),
        receive("jobs", 1),
        json!({"Command": "SetQueueAttributes", "QueueName": "jobs",
               "Attributes": {"DeadLetterQueue": "dead2", "MaxReceiveCount": 1}}),
        receive("jobs", 1),
        counts("dead1"),
        counts("dead2"),
    ]);
    assert_eq!(responses[6], json!({"ok": true, "Messages": []}));
    assert_eq!(responses[7]["Counts"]["Visible"], 0);
    assert_eq!(responses[8]["Counts"]["Visible"], 1);
}

#[test]
fn deleting_the_dead_letter_queue_turns_the_limit_off() {
    let responses = session(&[
        create("dead", json!({})),
        create(
            "jobs",
            json!({"DeadLetterQueue": "dead", "MaxReceiveCount": 1}),
        ),
        send("jobs", "x"),
        receive("jobs", 1),
        json!({"Command": "DeleteQueue", "QueueName": "dead"}),
        receive("jobs", 1),
    ]);
    assert_eq!(responses[4], ok());
    assert_eq!(responses[5]["Messages"][0]["ReceiveCount"], 2);
}
