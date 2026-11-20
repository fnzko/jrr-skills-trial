mod common;

use common::{Session, counts, delivered};
use serde_json::json;

fn pairs(items: &[(&str, &str)]) -> Vec<(String, String)> {
    items
        .iter()
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect()
}

/// A queue whose messages become visible again at once, so only the receive count matters.
fn with_dead_letter_queue(s: &mut Session, main: &str, dead: &str, max_receive_count: u64) {
    let fifo = main.ends_with(".fifo");
    s.create_with(dead, json!({"Fifo": fifo}));
    s.create_with(
        main,
        json!({"Fifo": fifo, "VisibilityTimeout": 0, "DeadLetterQueue": dead, "MaxReceiveCount": max_receive_count}),
    );
}

#[test]
fn a_message_is_moved_when_its_receive_count_reaches_the_maximum() {
    let mut s = Session::new();
    with_dead_letter_queue(&mut s, "main", "dead", 2);
    s.send_message("main", "x");
    assert_eq!(s.receive("main")["Messages"][0]["ReceiveCount"], json!(1));
    assert_eq!(s.receive("main")["Messages"][0]["ReceiveCount"], json!(2));
    s.tick(5);
    assert_eq!(s.receive("main"), json!({"ok": true, "Messages": []}));
    assert_eq!(s.counts("main"), counts(0, 0, 0));
    assert_eq!(s.counts("dead"), counts(1, 0, 0));
    assert_eq!(
        s.receive("dead"),
        json!({"ok": true, "Messages": [{
            "MessageId": "m-1",
            "ReceiptHandle": "r-3",
            "Body": "x",
            "ReceiveCount": 1,
            "SentAt": 0,
            "FirstReceivedAt": 5,
        }]})
    );
}

#[test]
fn a_moved_message_does_not_count_toward_max_messages() {
    let mut s = Session::new();
    with_dead_letter_queue(&mut s, "main", "dead", 1);
    s.send_message("main", "old");
    s.receive("main");
    s.send_message("main", "new");
    assert_eq!(delivered(&s.receive("main")), pairs(&[("m-2", "r-2")]));
    assert_eq!(s.counts("dead"), counts(1, 0, 0));
}

#[test]
fn a_moved_message_goes_to_the_end_of_the_dead_letter_queue() {
    let mut s = Session::new();
    with_dead_letter_queue(&mut s, "main", "dead", 1);
    s.send_message("dead", "already there");
    s.send_message("main", "moved");
    s.receive("main");
    s.receive("main");
    assert_eq!(
        delivered(&s.receive_up_to("dead", 10)),
        pairs(&[("m-1", "r-2"), ("m-2", "r-3")])
    );
}

#[test]
fn a_message_too_old_for_the_dead_letter_queue_is_dropped_on_arrival() {
    let mut s = Session::new();
    s.create_with("dead", json!({"RetentionSeconds": 60}));
    s.create_with(
        "main",
        json!({"VisibilityTimeout": 0, "DeadLetterQueue": "dead", "MaxReceiveCount": 1}),
    );
    s.send_message("main", "x");
    s.receive("main");
    s.tick(60);
    s.receive("main");
    assert_eq!(s.counts("main"), counts(0, 0, 0));
    assert_eq!(s.counts("dead"), counts(0, 0, 0));
}

#[test]
fn a_new_dead_letter_configuration_applies_from_the_next_receive() {
    let mut s = Session::new();
    s.create("dead");
    s.create_with("main", json!({"VisibilityTimeout": 0}));
    s.send_message("main", "x");
    s.receive("main");
    s.receive("main");
    s.ok(json!({"Command": "SetQueueAttributes", "QueueName": "main",
        "Attributes": {"DeadLetterQueue": "dead", "MaxReceiveCount": 2}}));
    assert_eq!(s.counts("main"), counts(1, 0, 0));
    assert_eq!(s.receive("main")["Messages"], json!([]));
    assert_eq!(s.counts("dead"), counts(1, 0, 0));
}

#[test]
fn deleting_the_dead_letter_queue_stops_moves() {
    let mut s = Session::new();
    with_dead_letter_queue(&mut s, "main", "dead", 1);
    s.send_message("main", "x");
    s.receive("main");
    s.ok(json!({"Command": "DeleteQueue", "QueueName": "dead"}));
    assert_eq!(s.receive("main")["Messages"][0]["ReceiveCount"], json!(2));
}

#[test]
fn in_a_fifo_queue_a_moved_head_unblocks_its_group_within_the_same_receive() {
    let mut s = Session::new();
    with_dead_letter_queue(&mut s, "m.fifo", "d.fifo", 1);
    s.send_grouped("m.fifo", "first", "g", "1");
    s.send_grouped("m.fifo", "second", "g", "2");
    s.receive("m.fifo");
    assert_eq!(delivered(&s.receive("m.fifo")), pairs(&[("m-2", "r-2")]));
    let moved = s.receive("d.fifo");
    assert_eq!(moved["Messages"][0]["MessageId"], json!("m-1"));
    assert_eq!(moved["Messages"][0]["GroupId"], json!("g"));
    assert_eq!(moved["Messages"][0]["SequenceNumber"], json!(1));
}

#[test]
fn a_moved_fifo_message_takes_no_deduplication_id_in_the_dead_letter_queue() {
    let mut s = Session::new();
    with_dead_letter_queue(&mut s, "m.fifo", "d.fifo", 1);
    s.send_grouped("m.fifo", "x", "g", "id");
    s.receive("m.fifo");
    s.receive("m.fifo");
    assert_eq!(
        s.send_grouped("d.fifo", "x", "g", "id"),
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2})
    );
}
