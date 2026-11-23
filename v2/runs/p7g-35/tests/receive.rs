mod common;

use common::Session;
use serde_json::{json, Value};

fn create(s: &mut Session, name: &str, attributes: Value) {
    s.ok(json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attributes}));
}

fn send(s: &mut Session, queue: &str, body: &str) {
    s.ok(json!({"Command": "SendMessage", "QueueName": queue, "Body": body}));
}

fn fifo_send(s: &mut Session, queue: &str, body: &str, group: &str) {
    s.ok(
        json!({"Command": "SendMessage", "QueueName": queue, "Body": body,
                "GroupId": group, "DeduplicationId": body}),
    );
}

fn receive(s: &mut Session, queue: &str, max: u64) -> Vec<Value> {
    let r = s.ok(json!({"Command": "ReceiveMessage", "QueueName": queue, "MaxMessages": max}));
    r["Messages"].as_array().unwrap().clone()
}

fn bodies(messages: &[Value]) -> Vec<&str> {
    messages
        .iter()
        .map(|m| m["Body"].as_str().unwrap())
        .collect()
}

fn counts(s: &mut Session, name: &str) -> Value {
    s.ok(json!({"Command": "GetQueueAttributes", "QueueName": name}))["Counts"].clone()
}

#[test]
fn receive_delivers_in_arrival_order_with_all_fields() {
    let mut s = Session::new();
    create(&mut s, "q", json!({}));
    send(&mut s, "q", "a");
    s.tick(5);
    send(&mut s, "q", "b");
    s.tick(2);
    let messages = receive(&mut s, "q", 10);
    assert_eq!(
        messages,
        vec![
            json!({"MessageId": "m-1", "ReceiptHandle": "r-1", "Body": "a", "ReceiveCount": 1, "SentAt": 0, "FirstReceivedAt": 7}),
            json!({"MessageId": "m-2", "ReceiptHandle": "r-2", "Body": "b", "ReceiveCount": 1, "SentAt": 5, "FirstReceivedAt": 7}),
        ]
    );
    assert_eq!(
        counts(&mut s, "q"),
        json!({"Visible": 0, "InFlight": 2, "Delayed": 0})
    );
    assert_eq!(receive(&mut s, "q", 10), Vec::<Value>::new());
}

#[test]
fn max_messages_defaults_to_one_and_is_validated() {
    let mut s = Session::new();
    create(&mut s, "q", json!({}));
    for body in ["a", "b", "c"] {
        send(&mut s, "q", body);
    }
    let r = s.ok(json!({"Command": "ReceiveMessage", "QueueName": "q"}));
    assert_eq!(bodies(r["Messages"].as_array().unwrap()), ["a"]);
    assert_eq!(bodies(&receive(&mut s, "q", 2)), ["b", "c"]);
    for bad in [json!(0), json!(11), json!("1"), json!(1.5)] {
        s.err(
            json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": bad}),
            "InvalidParameterValue",
        );
    }
    s.err(
        json!({"Command": "ReceiveMessage", "QueueName": "q", "VisibilityTimeout": 43201}),
        "InvalidParameterValue",
    );
    s.err(
        json!({"Command": "ReceiveMessage", "QueueName": "nope", "MaxMessages": 11}),
        "InvalidParameterValue",
    );
    s.err(
        json!({"Command": "ReceiveMessage", "QueueName": "nope"}),
        "QueueDoesNotExist",
    );
}

#[test]
fn a_message_becomes_visible_again_when_its_timeout_ends_and_keeps_its_place() {
    let mut s = Session::new();
    create(&mut s, "q", json!({"VisibilityTimeout": 10}));
    send(&mut s, "q", "a");
    send(&mut s, "q", "b");
    assert_eq!(bodies(&receive(&mut s, "q", 1)), ["a"]);
    s.tick(5);
    assert_eq!(bodies(&receive(&mut s, "q", 1)), ["b"]);
    s.tick(5);
    // a is visible again (t = 10) and precedes b, which is still in flight.
    let again = receive(&mut s, "q", 10);
    assert_eq!(bodies(&again), ["a"]);
    assert_eq!(again[0]["ReceiveCount"], 2);
    assert_eq!(again[0]["FirstReceivedAt"], 0);
    assert_eq!(again[0]["ReceiptHandle"], "r-3");
    s.tick(5);
    let mixed = receive(&mut s, "q", 10);
    assert_eq!(bodies(&mixed), ["b"]);
}

#[test]
fn the_visibility_timeout_can_be_chosen_per_receive() {
    let mut s = Session::new();
    create(&mut s, "q", json!({"VisibilityTimeout": 10}));
    send(&mut s, "q", "a");
    s.ok(json!({"Command": "ReceiveMessage", "QueueName": "q", "VisibilityTimeout": 2}));
    s.tick(2);
    assert_eq!(
        counts(&mut s, "q"),
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn a_zero_visibility_timeout_leaves_the_message_visible_and_the_handle_unusable() {
    let mut s = Session::new();
    create(&mut s, "q", json!({}));
    send(&mut s, "q", "a");
    let r = s.ok(json!({"Command": "ReceiveMessage", "QueueName": "q", "VisibilityTimeout": 0}));
    assert_eq!(r["Messages"][0]["ReceiptHandle"], "r-1");
    assert_eq!(counts(&mut s, "q")["Visible"], 1);
    s.err(
        json!({"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": "r-1"}),
        "ReceiptHandleExpired",
    );
    assert_eq!(receive(&mut s, "q", 1)[0]["ReceiveCount"], 2);
}

#[test]
fn delayed_messages_are_not_delivered_but_do_not_block_later_standard_messages() {
    let mut s = Session::new();
    create(&mut s, "q", json!({}));
    s.ok(json!({"Command": "SendMessage", "QueueName": "q", "Body": "late", "DelaySeconds": 10}));
    send(&mut s, "q", "now");
    assert_eq!(bodies(&receive(&mut s, "q", 10)), ["now"]);
    s.tick(10);
    assert_eq!(bodies(&receive(&mut s, "q", 10)), ["late"]);
}

#[test]
fn delete_message_removes_it_for_good() {
    let mut s = Session::new();
    create(&mut s, "q", json!({"VisibilityTimeout": 5}));
    send(&mut s, "q", "a");
    let handle = receive(&mut s, "q", 1)[0]["ReceiptHandle"].clone();
    s.ok(json!({"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": handle}));
    assert_eq!(
        counts(&mut s, "q"),
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
    s.err(
        json!({"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": handle}),
        "ReceiptHandleExpired",
    );
}

#[test]
fn receipt_handles_are_invalid_unless_they_were_issued_in_that_spelling() {
    let mut s = Session::new();
    create(&mut s, "q", json!({}));
    send(&mut s, "q", "a");
    receive(&mut s, "q", 1);
    for handle in [
        "",
        "r-0",
        "r-2",
        "r-01",
        "R-1",
        "r-1 ",
        " r-1",
        "r-+1",
        "1",
        "r-",
        "r--1",
        "r-99999999999999999999999",
    ] {
        for command in [
            json!({"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": handle}),
            json!({"Command": "ChangeMessageVisibility", "QueueName": "q", "ReceiptHandle": handle, "VisibilityTimeout": 1}),
        ] {
            s.err(command, "ReceiptHandleInvalid");
        }
    }
    s.err(
        json!({"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": 1}),
        "InvalidParameterValue",
    );
    s.err(
        json!({"Command": "DeleteMessage", "QueueName": "q"}),
        "MissingParameter",
    );
    s.err(
        json!({"Command": "DeleteMessage", "ReceiptHandle": "r-1"}),
        "MissingParameter",
    );
    s.err(
        json!({"Command": "DeleteMessage", "QueueName": "nope", "ReceiptHandle": "bad"}),
        "QueueDoesNotExist",
    );
    s.err(
        json!({"Command": "ChangeMessageVisibility", "QueueName": "q", "ReceiptHandle": "r-1"}),
        "MissingParameter",
    );
    s.err(json!({"Command": "ChangeMessageVisibility", "QueueName": "q", "ReceiptHandle": "r-1", "VisibilityTimeout": 43201}), "InvalidParameterValue");
    s.ok(json!({"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": "r-1"}));
}

#[test]
fn only_the_latest_handle_of_an_in_flight_message_is_usable() {
    let mut s = Session::new();
    create(&mut s, "q", json!({"VisibilityTimeout": 5}));
    create(&mut s, "other", json!({}));
    send(&mut s, "q", "a");
    receive(&mut s, "q", 1);
    s.tick(5);
    assert_eq!(receive(&mut s, "q", 1)[0]["ReceiptHandle"], "r-2");
    let delete =
        |h: &str, q: &str| json!({"Command": "DeleteMessage", "QueueName": q, "ReceiptHandle": h});
    s.err(delete("r-1", "q"), "ReceiptHandleExpired");
    s.err(delete("r-2", "other"), "ReceiptHandleExpired");
    s.tick(5);
    s.err(delete("r-2", "q"), "ReceiptHandleExpired");
    assert_eq!(counts(&mut s, "q")["Visible"], 1);
}

#[test]
fn handles_do_not_survive_purge_or_queue_deletion() {
    let mut s = Session::new();
    create(&mut s, "q", json!({}));
    send(&mut s, "q", "a");
    receive(&mut s, "q", 1);
    s.ok(json!({"Command": "PurgeQueue", "QueueName": "q"}));
    s.err(
        json!({"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": "r-1"}),
        "ReceiptHandleExpired",
    );
    send(&mut s, "q", "b");
    receive(&mut s, "q", 1);
    s.ok(json!({"Command": "DeleteQueue", "QueueName": "q"}));
    create(&mut s, "q", json!({}));
    s.err(
        json!({"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": "r-2"}),
        "ReceiptHandleExpired",
    );
}

#[test]
fn change_message_visibility_replaces_the_end_of_the_timeout() {
    let mut s = Session::new();
    create(&mut s, "q", json!({"VisibilityTimeout": 10}));
    send(&mut s, "q", "a");
    receive(&mut s, "q", 1);
    s.tick(8);
    let change = |t: u64| json!({"Command": "ChangeMessageVisibility", "QueueName": "q", "ReceiptHandle": "r-1", "VisibilityTimeout": t});
    s.ok(change(5));
    s.tick(4);
    assert_eq!(counts(&mut s, "q")["InFlight"], 1);
    s.tick(1);
    assert_eq!(counts(&mut s, "q")["Visible"], 1);
    s.err(change(5), "ReceiptHandleExpired");
    let again = receive(&mut s, "q", 1);
    assert_eq!(again[0]["ReceiveCount"], 2);
    s.ok(json!({"Command": "ChangeMessageVisibility", "QueueName": "q", "ReceiptHandle": "r-2", "VisibilityTimeout": 0}));
    assert_eq!(counts(&mut s, "q")["Visible"], 1);
}

#[test]
fn expired_messages_cannot_be_deleted() {
    let mut s = Session::new();
    create(
        &mut s,
        "q",
        json!({"RetentionSeconds": 60, "VisibilityTimeout": 1000}),
    );
    send(&mut s, "q", "a");
    receive(&mut s, "q", 1);
    s.tick(60);
    s.err(
        json!({"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": "r-1"}),
        "ReceiptHandleExpired",
    );
}

#[test]
fn fifo_delivers_one_message_per_group_and_blocks_groups_in_flight() {
    let mut s = Session::new();
    create(
        &mut s,
        "f.fifo",
        json!({"Fifo": true, "VisibilityTimeout": 10}),
    );
    for (body, group) in [
        ("a1", "a"),
        ("a2", "a"),
        ("b1", "b"),
        ("a3", "a"),
        ("c1", "c"),
    ] {
        fifo_send(&mut s, "f.fifo", body, group);
    }
    let first = receive(&mut s, "f.fifo", 10);
    assert_eq!(bodies(&first), ["a1", "b1", "c1"]);
    assert_eq!(first[0]["GroupId"], "a");
    assert_eq!(first[0]["SequenceNumber"], 1);
    assert_eq!(first[1]["SequenceNumber"], 3);
    assert_eq!(receive(&mut s, "f.fifo", 10), Vec::<Value>::new());
    s.ok(json!({"Command": "DeleteMessage", "QueueName": "f.fifo", "ReceiptHandle": first[0]["ReceiptHandle"]}));
    assert_eq!(bodies(&receive(&mut s, "f.fifo", 10)), ["a2"]);
    s.tick(10);
    // b1 and c1 are visible again; a2 is too, and a3 stays behind it.
    assert_eq!(bodies(&receive(&mut s, "f.fifo", 10)), ["a2", "b1", "c1"]);
}

#[test]
fn a_fifo_group_waits_while_its_earliest_message_is_in_flight() {
    let mut s = Session::new();
    create(&mut s, "f.fifo", json!({"Fifo": true}));
    fifo_send(&mut s, "f.fifo", "a1", "a");
    fifo_send(&mut s, "f.fifo", "a2", "a");
    let first = receive(&mut s, "f.fifo", 1);
    s.ok(json!({"Command": "ChangeMessageVisibility", "QueueName": "f.fifo", "ReceiptHandle": first[0]["ReceiptHandle"], "VisibilityTimeout": 0}));
    // a1 is visible again, so it goes first once more.
    assert_eq!(bodies(&receive(&mut s, "f.fifo", 10)), ["a1"]);
}

#[test]
fn messages_over_the_max_receive_count_move_to_the_dead_letter_queue() {
    let mut s = Session::new();
    create(&mut s, "dlq", json!({"RetentionSeconds": 1000}));
    create(
        &mut s,
        "q",
        json!({"VisibilityTimeout": 1, "DeadLetterQueue": "dlq", "MaxReceiveCount": 2}),
    );
    send(&mut s, "dlq", "already there");
    send(&mut s, "q", "poison");
    send(&mut s, "q", "fine");
    for expected_count in [1, 2] {
        let got = receive(&mut s, "q", 1);
        assert_eq!(got[0]["Body"], "poison");
        assert_eq!(got[0]["ReceiveCount"], expected_count);
        s.tick(1);
    }
    // The third examination moves poison away and delivers "fine" instead.
    let got = receive(&mut s, "q", 1);
    assert_eq!(bodies(&got), ["fine"]);
    assert_eq!(
        counts(&mut s, "q"),
        json!({"Visible": 0, "InFlight": 1, "Delayed": 0})
    );
    let moved = receive(&mut s, "dlq", 10);
    assert_eq!(
        moved,
        vec![
            json!({"MessageId": "m-1", "ReceiptHandle": "r-4", "Body": "already there", "ReceiveCount": 1, "SentAt": 0, "FirstReceivedAt": 2}),
            json!({"MessageId": "m-2", "ReceiptHandle": "r-5", "Body": "poison", "ReceiveCount": 1, "SentAt": 0, "FirstReceivedAt": 2}),
        ]
    );
}

#[test]
fn a_dead_lettered_message_is_visible_at_once_even_if_it_had_a_delay() {
    let mut s = Session::new();
    create(&mut s, "dlq", json!({"DelaySeconds": 500}));
    create(
        &mut s,
        "q",
        json!({"DeadLetterQueue": "dlq", "MaxReceiveCount": 1, "VisibilityTimeout": 0}),
    );
    send(&mut s, "q", "x");
    receive(&mut s, "q", 1);
    assert_eq!(receive(&mut s, "q", 1), Vec::<Value>::new());
    assert_eq!(
        counts(&mut s, "dlq"),
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(counts(&mut s, "q")["Visible"], 0);
}

#[test]
fn dead_letter_moves_ignore_the_message_limit_and_obey_the_target_retention() {
    let mut s = Session::new();
    create(&mut s, "dlq", json!({"RetentionSeconds": 100}));
    create(
        &mut s,
        "q",
        json!({"DeadLetterQueue": "dlq", "MaxReceiveCount": 1, "VisibilityTimeout": 0}),
    );
    send(&mut s, "q", "old");
    s.tick(100);
    send(&mut s, "q", "young");
    receive(&mut s, "q", 10);
    // "old" is 100 seconds old: gone from dlq on arrival. "young" stays.
    assert_eq!(receive(&mut s, "q", 10), Vec::<Value>::new());
    assert_eq!(
        counts(&mut s, "dlq"),
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn fifo_dead_letter_moves_take_new_sequence_numbers_and_free_the_group() {
    let mut s = Session::new();
    create(&mut s, "d.fifo", json!({"Fifo": true}));
    create(
        &mut s,
        "q.fifo",
        json!({"Fifo": true, "VisibilityTimeout": 0, "DeadLetterQueue": "d.fifo", "MaxReceiveCount": 1}),
    );
    fifo_send(&mut s, "d.fifo", "resident", "z");
    fifo_send(&mut s, "q.fifo", "first", "g");
    fifo_send(&mut s, "q.fifo", "second", "g");
    assert_eq!(bodies(&receive(&mut s, "q.fifo", 10)), ["first"]);
    // first is dead-lettered; its group is free again so second is delivered in the same call.
    let got = receive(&mut s, "q.fifo", 10);
    assert_eq!(bodies(&got), ["second"]);
    assert_eq!(got[0]["SequenceNumber"], 2);
    let moved = receive(&mut s, "d.fifo", 10);
    assert_eq!(bodies(&moved), ["resident", "first"]);
    assert_eq!(moved[1]["GroupId"], "g");
    assert_eq!(moved[1]["SequenceNumber"], 2);
    assert_eq!(moved[1]["ReceiveCount"], 1);
    assert_eq!(moved[1]["FirstReceivedAt"], 0);
}

#[test]
fn a_new_dead_letter_configuration_applies_from_the_next_receive() {
    let mut s = Session::new();
    create(&mut s, "dlq", json!({}));
    create(&mut s, "q", json!({"VisibilityTimeout": 0}));
    send(&mut s, "q", "x");
    receive(&mut s, "q", 1);
    receive(&mut s, "q", 1);
    s.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"DeadLetterQueue": "dlq", "MaxReceiveCount": 2}}));
    assert_eq!(counts(&mut s, "dlq")["Visible"], 0);
    assert_eq!(receive(&mut s, "q", 1), Vec::<Value>::new());
    assert_eq!(counts(&mut s, "dlq")["Visible"], 1);
}

#[test]
fn purge_removes_messages_in_every_state() {
    let mut s = Session::new();
    create(&mut s, "q", json!({}));
    send(&mut s, "q", "a");
    send(&mut s, "q", "b");
    s.ok(json!({"Command": "SendMessage", "QueueName": "q", "Body": "c", "DelaySeconds": 5}));
    receive(&mut s, "q", 1);
    s.ok(json!({"Command": "PurgeQueue", "QueueName": "q"}));
    assert_eq!(
        counts(&mut s, "q"),
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
    s.ok(json!({"Command": "PurgeQueue", "QueueName": "q"}));
}

#[test]
fn receiving_from_a_large_queue_with_everything_in_flight_stays_fast() {
    let mut s = Session::new();
    create(&mut s, "q", json!({}));
    for _ in 0..20_000 {
        s.ok(json!({"Command": "SendMessage", "QueueName": "q", "Body": "x"}));
    }
    assert_eq!(receive(&mut s, "q", 10).len(), 10);
    for _ in 0..50 {
        assert_eq!(receive(&mut s, "q", 10).len(), 10);
    }
    assert_eq!(counts(&mut s, "q")["InFlight"], 510);
}
