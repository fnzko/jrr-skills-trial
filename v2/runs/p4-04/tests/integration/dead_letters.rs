use serde_json::{Value, json};

use crate::support::{Session, counts};

fn receive_one(queue: &str) -> Value {
    json!({"Command": "ReceiveMessage", "QueueName": queue, "MaxMessages": 1})
}

#[test]
fn messages_over_the_receive_limit_move_to_the_dead_letter_queue() {
    let mut session = Session::default();
    session.create("dlq", json!({}));
    session.create(
        "q",
        json!({"VisibilityTimeout": 0, "DeadLetterQueue": "dlq", "MaxReceiveCount": 2}),
    );
    session.send("q", "a");
    session.tick(5);
    session.send("q", "b");
    assert_eq!(session.receive_ids("q"), ["m-1", "m-2"]);
    assert_eq!(session.receive_ids("q"), ["m-1", "m-2"]);
    session.tick(1);
    assert_eq!(session.receive("q"), json!({"ok": true, "Messages": []}));
    assert_eq!(session.counts("q"), counts(0, 0, 0));
    assert_eq!(session.counts("dlq"), counts(2, 0, 0));
    session.tick(1);
    assert_eq!(
        session.receive("dlq"),
        json!({"ok": true, "Messages": [
            {"MessageId": "m-1", "ReceiptHandle": "r-5", "Body": "a", "ReceiveCount": 1,
             "SentAt": 0, "FirstReceivedAt": 7},
            {"MessageId": "m-2", "ReceiptHandle": "r-6", "Body": "b", "ReceiveCount": 1,
             "SentAt": 5, "FirstReceivedAt": 7},
        ]})
    );
}

#[test]
fn moved_messages_do_not_count_toward_max_messages() {
    let mut session = Session::default();
    session.create("dlq", json!({}));
    session.create(
        "q",
        json!({"VisibilityTimeout": 0, "DeadLetterQueue": "dlq", "MaxReceiveCount": 1}),
    );
    session.send("q", "a");
    assert_eq!(
        session.run(receive_one("q"))["Messages"][0]["MessageId"],
        json!("m-1")
    );
    session.send("q", "b");
    session.send("q", "c");
    let reply = session.run(receive_one("q"));
    assert_eq!(reply["Messages"].as_array().map(Vec::len), Some(1));
    assert_eq!(reply["Messages"][0]["MessageId"], json!("m-2"));
    assert_eq!(session.counts("dlq"), counts(1, 0, 0));
    assert_eq!(session.counts("q"), counts(2, 0, 0));
}

#[test]
fn fifo_moves_renumber_and_unblock_the_group() {
    let mut session = Session::default();
    session.create("dlq.fifo", json!({"Fifo": true}));
    session.create(
        "q.fifo",
        json!({"Fifo": true, "VisibilityTimeout": 0, "DeadLetterQueue": "dlq.fifo", "MaxReceiveCount": 1}),
    );
    let send = |queue: &str, dedup: &str| {
        json!({"Command": "SendMessage", "QueueName": queue, "Body": dedup, "GroupId": "g",
               "DeduplicationId": dedup})
    };
    for dedup in ["d1", "d2"] {
        session.run(send("dlq.fifo", dedup));
    }
    session.run(send("q.fifo", "a"));
    session.run(send("q.fifo", "b"));
    assert_eq!(session.receive_ids("q.fifo"), ["m-3"]);
    // m-3 moves out, so m-4 heads group g within the same receive.
    let reply = session.receive("q.fifo");
    assert_eq!(
        reply["Messages"],
        json!([{"MessageId": "m-4", "ReceiptHandle": "r-2", "Body": "b", "ReceiveCount": 1,
                "SentAt": 0, "FirstReceivedAt": 0, "GroupId": "g", "SequenceNumber": 2}])
    );
    assert_eq!(session.counts("dlq.fifo"), counts(3, 0, 0));
    // The move recorded no deduplication id in the dead-letter queue.
    assert_eq!(
        session.run(send("dlq.fifo", "a")),
        json!({"ok": true, "MessageId": "m-5", "SequenceNumber": 4})
    );
    // Group g in the dead-letter queue: m-1, m-2, then the moved m-3.
    assert_eq!(session.receive_ids("dlq.fifo"), ["m-1"]);
    session.delete("dlq.fifo", "r-3");
    assert_eq!(session.receive_ids("dlq.fifo"), ["m-2"]);
    session.delete("dlq.fifo", "r-4");
    let reply = session.receive("dlq.fifo");
    assert_eq!(reply["Messages"][0]["MessageId"], json!("m-3"));
    assert_eq!(reply["Messages"][0]["SequenceNumber"], json!(3));
    assert_eq!(reply["Messages"][0]["GroupId"], json!("g"));
}

#[test]
fn old_handles_expire_when_a_message_moves() {
    let mut session = Session::default();
    session.create("dlq", json!({}));
    session.create("q", json!({"DeadLetterQueue": "dlq", "MaxReceiveCount": 1}));
    session.send("q", "a");
    session.receive_ids("q");
    let change = json!({"Command": "ChangeMessageVisibility", "QueueName": "q", "ReceiptHandle": "r-1",
                        "VisibilityTimeout": 0});
    assert_eq!(session.run(change), json!({"ok": true}));
    assert_eq!(session.receive_ids("q"), Vec::<String>::new());
    for queue in ["q", "dlq"] {
        let delete =
            json!({"Command": "DeleteMessage", "QueueName": queue, "ReceiptHandle": "r-1"});
        session.fails(delete, "ReceiptHandleExpired");
    }
}

#[test]
fn a_message_too_old_for_the_dead_letter_queue_is_removed_on_arrival() {
    let mut session = Session::default();
    session.create("dlq", json!({"RetentionSeconds": 60}));
    session.create(
        "q",
        json!({"VisibilityTimeout": 0, "DeadLetterQueue": "dlq", "MaxReceiveCount": 1}),
    );
    session.send("q", "old");
    session.tick(59);
    session.send("q", "young");
    session.receive_ids("q");
    session.tick(1);
    assert_eq!(session.receive_ids("q"), Vec::<String>::new());
    assert_eq!(session.counts("dlq"), counts(1, 0, 0));
    assert_eq!(session.receive_ids("dlq"), ["m-2"]);
}

#[test]
fn dead_letter_configuration_changes_take_effect_on_the_next_receive() {
    let mut session = Session::default();
    session.create("one", json!({}));
    session.create("two", json!({}));
    session.create("q", json!({"VisibilityTimeout": 0}));
    session.send("q", "a");
    session.send("q", "b");
    assert_eq!(session.receive_ids("q"), ["m-1", "m-2"]);
    let set = |target: &str| {
        json!({"Command": "SetQueueAttributes", "QueueName": "q",
               "Attributes": {"DeadLetterQueue": target, "MaxReceiveCount": 1}})
    };
    assert_eq!(session.run(set("one")), json!({"ok": true}));
    assert_eq!(session.run(receive_one("q"))["Messages"], json!([]));
    assert_eq!(session.counts("one"), counts(2, 0, 0));

    session.send("q", "c");
    session.receive_ids("q");
    assert_eq!(session.run(set("two")), json!({"ok": true}));
    // Deleting the dead-letter queue removes the configuration.
    assert_eq!(
        session.run(json!({"Command": "DeleteQueue", "QueueName": "two"})),
        json!({"ok": true})
    );
    assert_eq!(session.receive_ids("q"), ["m-3"]);
    assert_eq!(
        session.run(json!({"Command": "GetQueueAttributes", "QueueName": "q"}))["Attributes"],
        json!({"VisibilityTimeout": 0, "DelaySeconds": 0, "RetentionSeconds": 345600,
               "Fifo": false, "ContentBasedDeduplication": false})
    );
}

#[test]
fn moves_ignore_the_message_limit() {
    let mut session = Session::default();
    session.create("dlq", json!({}));
    session.create(
        "q",
        json!({"VisibilityTimeout": 0, "DeadLetterQueue": "dlq", "MaxReceiveCount": 1}),
    );
    session.send("q", "moving");
    session.receive_ids("q");
    let line = br#"{"Command":"SendMessage","QueueName":"dlq","Body":"x"}"#;
    for _ in 0..100_000 {
        assert_eq!(session.line(line)["ok"], json!(true));
    }
    assert_eq!(session.receive_ids("q"), Vec::<String>::new());
    assert_eq!(session.counts("dlq"), counts(100_001, 0, 0));
}
