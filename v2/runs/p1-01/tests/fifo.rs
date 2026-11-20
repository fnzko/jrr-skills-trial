mod common;

use common::{Session, counts, delivered};
use serde_json::json;

fn pairs(items: &[(&str, &str)]) -> Vec<(String, String)> {
    items
        .iter()
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect()
}

fn fifo(s: &mut Session, name: &str) {
    s.create_with(name, json!({"Fifo": true}));
}

#[test]
fn fifo_sends_report_sequence_numbers_and_deliveries_carry_group_and_sequence() {
    let mut s = Session::new();
    fifo(&mut s, "f.fifo");
    assert_eq!(
        s.send_grouped("f.fifo", "a", "g", "d1"),
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
    assert_eq!(
        s.send_grouped("f.fifo", "b", "h", "d2"),
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2})
    );
    assert_eq!(
        s.receive("f.fifo"),
        json!({"ok": true, "Messages": [{
            "MessageId": "m-1",
            "ReceiptHandle": "r-1",
            "Body": "a",
            "ReceiveCount": 1,
            "SentAt": 0,
            "FirstReceivedAt": 0,
            "GroupId": "g",
            "SequenceNumber": 1,
        }]})
    );
}

#[test]
fn fifo_receive_delivers_one_message_per_group_and_blocks_a_group_while_its_head_is_in_flight() {
    let mut s = Session::new();
    fifo(&mut s, "f.fifo");
    s.send_grouped("f.fifo", "a1", "g1", "1");
    s.send_grouped("f.fifo", "a2", "g1", "2");
    s.send_grouped("f.fifo", "b1", "g2", "3");
    assert_eq!(
        delivered(&s.receive_up_to("f.fifo", 10)),
        pairs(&[("m-1", "r-1"), ("m-3", "r-2")])
    );
    assert_eq!(delivered(&s.receive_up_to("f.fifo", 10)), []);
    assert_eq!(s.delete_message("f.fifo", "r-1"), json!({"ok": true}));
    assert_eq!(
        delivered(&s.receive_up_to("f.fifo", 10)),
        pairs(&[("m-2", "r-3")])
    );
}

#[test]
fn a_delayed_group_head_blocks_later_messages_of_its_group() {
    // FIFO sends take no DelaySeconds, so the queue's own delay is what delays a head.
    let mut s = Session::new();
    s.create_with("f.fifo", json!({"Fifo": true, "DelaySeconds": 10}));
    s.send_grouped("f.fifo", "a1", "g", "1");
    s.ok(json!({"Command": "SetQueueAttributes", "QueueName": "f.fifo", "Attributes": {"DelaySeconds": 0}}));
    s.send_grouped("f.fifo", "a2", "g", "2");
    s.send_grouped("f.fifo", "b", "h", "3");
    assert_eq!(
        delivered(&s.receive_up_to("f.fifo", 10)),
        pairs(&[("m-3", "r-1")])
    );
    assert_eq!(s.counts("f.fifo"), counts(1, 1, 1));
    s.tick(10);
    assert_eq!(
        delivered(&s.receive_up_to("f.fifo", 10)),
        pairs(&[("m-1", "r-2")])
    );
}

#[test]
fn fifo_send_applies_its_rules_in_order() {
    let mut s = Session::new();
    fifo(&mut s, "f.fifo");
    s.create_with(
        "c.fifo",
        json!({"Fifo": true, "ContentBasedDeduplication": true}),
    );
    let send = |queue: &str, extra: serde_json::Value| {
        let mut command = json!({"Command": "SendMessage", "QueueName": queue, "Body": "x"});
        for (key, value) in extra.as_object().unwrap() {
            command[key] = value.clone();
        }
        command
    };
    assert_eq!(
        s.err(send("f.fifo", json!({"DelaySeconds": 0}))),
        "UnsupportedOperation"
    );
    assert_eq!(
        s.err(send(
            "f.fifo",
            json!({"DelaySeconds": 0, "DeduplicationId": "d"})
        )),
        "UnsupportedOperation"
    );
    assert_eq!(
        s.err(send("f.fifo", json!({"DeduplicationId": "d"}))),
        "MissingParameter"
    );
    assert_eq!(
        s.err(send("f.fifo", json!({"GroupId": "g"}))),
        "MissingParameter"
    );
    assert_eq!(s.err(send("c.fifo", json!({}))), "MissingParameter");
    s.ok(send("c.fifo", json!({"GroupId": "g"})));
}

#[test]
fn a_duplicate_send_returns_the_original_ids_and_uses_up_no_numbers() {
    let mut s = Session::new();
    fifo(&mut s, "f.fifo");
    s.send_grouped("f.fifo", "first", "g", "x");
    s.tick(100);
    assert_eq!(
        s.send_grouped("f.fifo", "second", "other-group", "x"),
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
    assert_eq!(
        s.send_grouped("f.fifo", "third", "g", "y"),
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2})
    );
    assert_eq!(s.counts("f.fifo"), counts(2, 0, 0));
}

#[test]
fn the_deduplication_window_is_300_seconds_and_a_duplicate_does_not_extend_it() {
    let mut s = Session::new();
    fifo(&mut s, "f.fifo");
    s.send_grouped("f.fifo", "a", "g", "x");
    s.tick(299);
    assert_eq!(
        s.send_grouped("f.fifo", "a", "g", "x")["MessageId"],
        json!("m-1")
    );
    s.tick(1);
    assert_eq!(
        s.send_grouped("f.fifo", "a", "g", "x"),
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2})
    );
    s.tick(299);
    assert_eq!(
        s.send_grouped("f.fifo", "a", "g", "x")["MessageId"],
        json!("m-2")
    );
}

#[test]
fn a_deduplication_id_outlives_its_message() {
    let mut s = Session::new();
    fifo(&mut s, "f.fifo");
    s.send_grouped("f.fifo", "a", "g", "x");
    s.receive("f.fifo");
    s.delete_message("f.fifo", "r-1");
    assert_eq!(
        s.send_grouped("f.fifo", "a", "g", "x")["MessageId"],
        json!("m-1")
    );
    assert_eq!(s.counts("f.fifo"), counts(0, 0, 0));
    s.send_grouped("f.fifo", "b", "g", "y");
    s.ok(json!({"Command": "PurgeQueue", "QueueName": "f.fifo"}));
    assert_eq!(
        s.send_grouped("f.fifo", "b", "g", "y")["MessageId"],
        json!("m-2")
    );
    assert_eq!(
        s.send_grouped("f.fifo", "c", "g", "z"),
        json!({"ok": true, "MessageId": "m-3", "SequenceNumber": 3})
    );
}

#[test]
fn deduplication_ids_are_per_queue() {
    let mut s = Session::new();
    fifo(&mut s, "a.fifo");
    fifo(&mut s, "b.fifo");
    s.send_grouped("a.fifo", "x", "g", "d");
    assert_eq!(
        s.send_grouped("b.fifo", "x", "g", "d")["MessageId"],
        json!("m-2")
    );
}

#[test]
fn content_based_deduplication_uses_the_body_unless_an_id_is_given() {
    let mut s = Session::new();
    s.create_with(
        "c.fifo",
        json!({"Fifo": true, "ContentBasedDeduplication": true}),
    );
    let send = |body: &str, dedup: Option<&str>| {
        let mut command =
            json!({"Command": "SendMessage", "QueueName": "c.fifo", "Body": body, "GroupId": "g"});
        if let Some(id) = dedup {
            command["DeduplicationId"] = json!(id);
        }
        command
    };
    assert_eq!(s.ok(send("same", None))["MessageId"], json!("m-1"));
    assert_eq!(s.ok(send("same", None))["MessageId"], json!("m-1"));
    assert_eq!(
        s.ok(send("same", Some("explicit")))["MessageId"],
        json!("m-2")
    );
    assert_eq!(s.ok(send("other", None))["MessageId"], json!("m-3"));
    // An explicit id and a body used as an id are the same kind of key.
    assert_eq!(s.ok(send("explicit", None))["MessageId"], json!("m-2"));
}

#[test]
fn sequence_numbers_continue_after_a_purge() {
    let mut s = Session::new();
    fifo(&mut s, "f.fifo");
    s.send_grouped("f.fifo", "a", "g", "1");
    s.send_grouped("f.fifo", "b", "g", "2");
    s.ok(json!({"Command": "PurgeQueue", "QueueName": "f.fifo"}));
    assert_eq!(
        s.send_grouped("f.fifo", "c", "g", "3")["SequenceNumber"],
        json!(3)
    );
}
