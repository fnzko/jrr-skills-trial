mod common;

use common::Session;
use serde_json::{Value, json};

fn create(s: &mut Session, name: &str, attributes: Value) {
    let line = json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attributes});
    s.ok(&line.to_string());
}

fn send(s: &mut Session, queue: &str, body: &str) {
    s.ok(&json!({"Command": "SendMessage", "QueueName": queue, "Body": body}).to_string());
}

fn fifo_send(s: &mut Session, queue: &str, group: &str, body: &str) {
    let line = json!({"Command": "SendMessage", "QueueName": queue, "Body": body,
                      "GroupId": group, "DeduplicationId": body});
    s.ok(&line.to_string());
}

fn tick(s: &mut Session, seconds: u32) {
    s.ok(&format!(r#"{{"Command": "Tick", "Seconds": {seconds}}}"#));
}

fn receive(s: &mut Session, queue: &str, max: u32) -> Vec<Value> {
    let line = json!({"Command": "ReceiveMessage", "QueueName": queue, "MaxMessages": max});
    s.ok(&line.to_string())["Messages"]
        .as_array()
        .unwrap()
        .clone()
}

fn bodies(messages: &[Value]) -> Vec<&str> {
    messages
        .iter()
        .map(|m| m["Body"].as_str().unwrap())
        .collect()
}

fn counts(s: &mut Session, queue: &str) -> Value {
    let line = json!({"Command": "GetQueueAttributes", "QueueName": queue});
    s.ok(&line.to_string())["Counts"].clone()
}

#[test]
fn receive_returns_messages_in_arrival_order_with_all_members() {
    let mut s = Session::new();
    create(&mut s, "q", json!({}));
    send(&mut s, "q", "first");
    tick(&mut s, 4);
    send(&mut s, "q", "second");
    tick(&mut s, 3);
    assert_eq!(
        s.ok(r#"{"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 10}"#),
        json!({"Messages": [
            {"MessageId": "m-1", "ReceiptHandle": "r-1", "Body": "first",
             "ReceiveCount": 1, "SentAt": 0, "FirstReceivedAt": 7},
            {"MessageId": "m-2", "ReceiptHandle": "r-2", "Body": "second",
             "ReceiveCount": 1, "SentAt": 4, "FirstReceivedAt": 7},
        ]})
    );
    assert_eq!(
        counts(&mut s, "q"),
        json!({"Visible": 0, "InFlight": 2, "Delayed": 0})
    );
}

#[test]
fn receive_on_an_empty_queue_returns_an_empty_array() {
    let mut s = Session::new();
    create(&mut s, "q", json!({}));
    assert_eq!(
        s.ok(r#"{"Command": "ReceiveMessage", "QueueName": "q"}"#),
        json!({"Messages": []})
    );
}

#[test]
fn max_messages_defaults_to_one_and_is_range_checked() {
    let mut s = Session::new();
    create(&mut s, "q", json!({}));
    for body in ["a", "b", "c", "d"] {
        send(&mut s, "q", body);
    }
    assert_eq!(bodies(&receive_default(&mut s)), ["a"]);
    assert_eq!(bodies(&receive(&mut s, "q", 2)), ["b", "c"]);
    for max in ["0", "11", "-1", "1.0", "\"1\""] {
        let line =
            format!(r#"{{"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": {max}}}"#);
        assert_eq!(s.err(&line), "InvalidParameterValue", "{max}");
    }
    assert_eq!(
        s.err(r#"{"Command": "ReceiveMessage", "QueueName": "q", "VisibilityTimeout": 43201}"#),
        "InvalidParameterValue"
    );
    assert_eq!(
        s.err(r#"{"Command": "ReceiveMessage", "QueueName": "nope", "MaxMessages": 0}"#),
        "InvalidParameterValue"
    );
    assert_eq!(
        s.err(r#"{"Command": "ReceiveMessage", "QueueName": "nope"}"#),
        "QueueDoesNotExist"
    );
    assert_eq!(
        s.err(r#"{"Command": "ReceiveMessage"}"#),
        "MissingParameter"
    );
}

fn receive_default(s: &mut Session) -> Vec<Value> {
    s.ok(r#"{"Command": "ReceiveMessage", "QueueName": "q"}"#)["Messages"]
        .as_array()
        .unwrap()
        .clone()
}

#[test]
fn delayed_messages_are_not_delivered_until_their_delay_ends() {
    let mut s = Session::new();
    create(&mut s, "q", json!({"DelaySeconds": 10}));
    send(&mut s, "q", "a");
    tick(&mut s, 9);
    assert!(receive(&mut s, "q", 10).is_empty());
    tick(&mut s, 1);
    assert_eq!(bodies(&receive(&mut s, "q", 10)), ["a"]);
}

#[test]
fn a_message_returns_after_its_visibility_timeout_in_its_original_place() {
    let mut s = Session::new();
    create(&mut s, "q", json!({"VisibilityTimeout": 30}));
    send(&mut s, "q", "a");
    send(&mut s, "q", "b");
    tick(&mut s, 1);
    assert_eq!(bodies(&receive(&mut s, "q", 1)), ["a"]);
    tick(&mut s, 1);
    assert_eq!(bodies(&receive(&mut s, "q", 1)), ["b"]);
    assert!(receive(&mut s, "q", 10).is_empty());
    tick(&mut s, 29);
    // a was received at 1 and is visible from 31; b from 32.
    assert_eq!(bodies(&receive(&mut s, "q", 10)), ["a"]);
    tick(&mut s, 1);
    let again = receive(&mut s, "q", 10);
    assert_eq!(bodies(&again), ["b"]);
    assert_eq!(again[0]["ReceiveCount"], 2);
    assert_eq!(again[0]["FirstReceivedAt"], 2);
    assert_eq!(again[0]["ReceiptHandle"], "r-4");
}

#[test]
fn receive_visibility_timeout_overrides_the_queue_and_zero_makes_it_visible_at_once() {
    let mut s = Session::new();
    create(&mut s, "q", json!({"VisibilityTimeout": 100}));
    send(&mut s, "q", "a");
    let line = r#"{"Command": "ReceiveMessage", "QueueName": "q", "VisibilityTimeout": 0}"#;
    assert_eq!(s.ok(line)["Messages"][0]["ReceiveCount"], 1);
    assert_eq!(s.ok(line)["Messages"][0]["ReceiveCount"], 2);
    assert_eq!(
        counts(&mut s, "q"),
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
    s.ok(r#"{"Command": "ReceiveMessage", "QueueName": "q", "VisibilityTimeout": 5}"#);
    tick(&mut s, 4);
    assert_eq!(
        counts(&mut s, "q"),
        json!({"Visible": 0, "InFlight": 1, "Delayed": 0})
    );
    tick(&mut s, 1);
    assert_eq!(
        counts(&mut s, "q"),
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn a_changed_queue_visibility_timeout_applies_to_later_deliveries_only() {
    let mut s = Session::new();
    create(&mut s, "q", json!({"VisibilityTimeout": 10}));
    send(&mut s, "q", "a");
    receive(&mut s, "q", 1);
    s.ok(r#"{"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"VisibilityTimeout": 100}}"#);
    tick(&mut s, 10);
    assert_eq!(bodies(&receive(&mut s, "q", 1)), ["a"]);
    tick(&mut s, 99);
    assert!(receive(&mut s, "q", 1).is_empty());
}

#[test]
fn fifo_delivers_at_most_one_message_per_group_per_receive() {
    let mut s = Session::new();
    create(&mut s, "q.fifo", json!({"Fifo": true}));
    fifo_send(&mut s, "q.fifo", "g1", "a");
    fifo_send(&mut s, "q.fifo", "g1", "b");
    fifo_send(&mut s, "q.fifo", "g2", "c");
    fifo_send(&mut s, "q.fifo", "g2", "d");
    let got = receive(&mut s, "q.fifo", 10);
    assert_eq!(bodies(&got), ["a", "c"]);
    assert_eq!(got[0]["GroupId"], "g1");
    assert_eq!(got[0]["SequenceNumber"], 1);
    assert_eq!(got[1]["GroupId"], "g2");
    assert_eq!(got[1]["SequenceNumber"], 3);
    // Both groups are blocked by the in-flight heads.
    assert!(receive(&mut s, "q.fifo", 10).is_empty());
    tick(&mut s, 30);
    // The heads are visible again and still come first.
    assert_eq!(bodies(&receive(&mut s, "q.fifo", 10)), ["a", "c"]);
}

#[test]
fn fifo_group_is_blocked_by_an_earlier_delayed_message() {
    let mut s = Session::new();
    create(&mut s, "q.fifo", json!({"Fifo": true, "DelaySeconds": 5}));
    fifo_send(&mut s, "q.fifo", "g", "a");
    s.ok(r#"{"Command": "SetQueueAttributes", "QueueName": "q.fifo", "Attributes": {"DelaySeconds": 0}}"#);
    fifo_send(&mut s, "q.fifo", "g", "b");
    fifo_send(&mut s, "q.fifo", "h", "c");
    assert_eq!(bodies(&receive(&mut s, "q.fifo", 10)), ["c"]);
    tick(&mut s, 5);
    assert_eq!(bodies(&receive(&mut s, "q.fifo", 10)), ["a"]);
}

#[test]
fn max_messages_limits_a_fifo_receive_too() {
    let mut s = Session::new();
    create(&mut s, "q.fifo", json!({"Fifo": true}));
    for (group, body) in [("g1", "a"), ("g2", "b"), ("g3", "c")] {
        fifo_send(&mut s, "q.fifo", group, body);
    }
    assert_eq!(bodies(&receive(&mut s, "q.fifo", 2)), ["a", "b"]);
    assert_eq!(bodies(&receive(&mut s, "q.fifo", 2)), ["c"]);
}

fn dead_letter_pair(s: &mut Session, max_receive_count: u32, source: Value, target: Value) {
    create(s, "dead", target);
    let mut attributes = source;
    attributes["DeadLetterQueue"] = json!("dead");
    attributes["MaxReceiveCount"] = json!(max_receive_count);
    create(s, "q", attributes);
}

#[test]
fn a_message_received_too_often_moves_to_the_dead_letter_queue() {
    let mut s = Session::new();
    dead_letter_pair(&mut s, 2, json!({"VisibilityTimeout": 10}), json!({}));
    send(&mut s, "q", "poison");
    tick(&mut s, 3);
    receive(&mut s, "q", 1);
    tick(&mut s, 10);
    assert_eq!(receive(&mut s, "q", 1)[0]["ReceiveCount"], 2);
    tick(&mut s, 10);
    assert!(receive(&mut s, "q", 1).is_empty());
    assert_eq!(
        counts(&mut s, "q"),
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(
        counts(&mut s, "dead"),
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
    // It starts over there: same id, body and sent time; fresh counters.
    tick(&mut s, 1);
    assert_eq!(
        receive(&mut s, "dead", 1),
        [
            json!({"MessageId": "m-1", "ReceiptHandle": "r-3", "Body": "poison",
                "ReceiveCount": 1, "SentAt": 0, "FirstReceivedAt": 24})
        ]
    );
}

#[test]
fn dead_lettered_messages_do_not_count_toward_max_messages() {
    let mut s = Session::new();
    dead_letter_pair(&mut s, 1, json!({"VisibilityTimeout": 0}), json!({}));
    send(&mut s, "q", "old");
    receive(&mut s, "q", 1);
    send(&mut s, "q", "new1");
    send(&mut s, "q", "new2");
    assert_eq!(bodies(&receive(&mut s, "q", 1)), ["new1"]);
    assert_eq!(bodies(&receive(&mut s, "dead", 10)), ["old"]);
}

#[test]
fn a_message_too_old_for_the_dead_letter_queue_is_removed_on_arrival() {
    let mut s = Session::new();
    dead_letter_pair(
        &mut s,
        1,
        json!({"VisibilityTimeout": 0}),
        json!({"RetentionSeconds": 60}),
    );
    send(&mut s, "q", "a");
    receive(&mut s, "q", 1);
    tick(&mut s, 100);
    assert!(receive(&mut s, "q", 1).is_empty());
    assert_eq!(
        counts(&mut s, "dead"),
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn dead_letter_move_ignores_the_message_limit_of_the_target() {
    let mut s = Session::new();
    dead_letter_pair(&mut s, 1, json!({"VisibilityTimeout": 0}), json!({}));
    send(&mut s, "q", "a");
    receive(&mut s, "q", 1);
    let line = r#"{"Command": "SendMessage", "QueueName": "dead", "Body": "x"}"#;
    for _ in 0..100_000 {
        s.ok(line);
    }
    assert_eq!(s.err(line), "LimitExceeded");
    assert!(receive(&mut s, "q", 1).is_empty());
    assert_eq!(counts(&mut s, "dead")["Visible"], 100_001);
}

#[test]
fn fifo_dead_letter_move_keeps_the_group_and_takes_a_new_sequence_number() {
    let mut s = Session::new();
    create(&mut s, "dead.fifo", json!({"Fifo": true}));
    create(
        &mut s,
        "q.fifo",
        json!({"Fifo": true, "VisibilityTimeout": 0, "DeadLetterQueue": "dead.fifo", "MaxReceiveCount": 1}),
    );
    fifo_send(&mut s, "dead.fifo", "other", "direct");
    fifo_send(&mut s, "q.fifo", "g", "a");
    receive(&mut s, "q.fifo", 1);
    // "a" is dead-lettered, which frees the group for "b" in the same receive.
    fifo_send(&mut s, "q.fifo", "g", "b");
    let got = receive(&mut s, "q.fifo", 10);
    assert_eq!(bodies(&got), ["b"]);
    assert_eq!(got[0]["SequenceNumber"], 2);
    let dead = receive(&mut s, "dead.fifo", 10);
    assert_eq!(bodies(&dead), ["direct", "a"]);
    assert_eq!(dead[1]["GroupId"], "g");
    assert_eq!(dead[1]["SequenceNumber"], 2);
    assert_eq!(dead[1]["ReceiveCount"], 1);
}

#[test]
fn a_new_dead_letter_configuration_applies_from_the_next_receive() {
    let mut s = Session::new();
    create(&mut s, "dead", json!({}));
    create(&mut s, "q", json!({"VisibilityTimeout": 0}));
    send(&mut s, "q", "a");
    for _ in 0..3 {
        receive(&mut s, "q", 1);
    }
    s.ok(r#"{"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"DeadLetterQueue": "dead", "MaxReceiveCount": 3}}"#);
    assert!(receive(&mut s, "q", 1).is_empty());
    assert_eq!(bodies(&receive(&mut s, "dead", 1)), ["a"]);
}

#[test]
fn deleting_the_dead_letter_queue_turns_dead_lettering_off() {
    let mut s = Session::new();
    dead_letter_pair(&mut s, 1, json!({"VisibilityTimeout": 0}), json!({}));
    send(&mut s, "q", "a");
    receive(&mut s, "q", 1);
    s.ok(r#"{"Command": "DeleteQueue", "QueueName": "dead"}"#);
    assert_eq!(bodies(&receive(&mut s, "q", 1)), ["a"]);
}
