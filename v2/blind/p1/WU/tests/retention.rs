mod common;

use common::Miniq;
use serde_json::{Value, json};

fn create(miniq: &mut Miniq, name: &str, attributes: Value) {
    miniq.ok(json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attributes}));
}

fn send(miniq: &mut Miniq, queue: &str, body: &str) {
    miniq.ok(json!({"Command": "SendMessage", "QueueName": queue, "Body": body}));
}

fn tick(miniq: &mut Miniq, seconds: u64) {
    miniq.ok(json!({"Command": "Tick", "Seconds": seconds}));
}

fn set(miniq: &mut Miniq, queue: &str, attributes: Value) {
    miniq
        .ok(json!({"Command": "SetQueueAttributes", "QueueName": queue, "Attributes": attributes}));
}

fn counts(miniq: &mut Miniq, queue: &str) -> Value {
    miniq.ok(json!({"Command": "GetQueueAttributes", "QueueName": queue}))["Counts"].clone()
}

fn total(miniq: &mut Miniq, queue: &str) -> u64 {
    counts(miniq, queue)
        .as_object()
        .unwrap()
        .values()
        .map(|n| n.as_u64().unwrap())
        .sum()
}

#[test]
fn a_message_is_removed_exactly_at_sent_at_plus_retention() {
    let mut miniq = Miniq::new();
    create(&mut miniq, "q", json!({"RetentionSeconds": 60}));
    tick(&mut miniq, 10);
    send(&mut miniq, "q", "a");
    tick(&mut miniq, 59);
    assert_eq!(total(&mut miniq, "q"), 1);
    tick(&mut miniq, 1);
    assert_eq!(total(&mut miniq, "q"), 0);
}

#[test]
fn retention_removes_messages_in_every_state() {
    let mut miniq = Miniq::new();
    create(
        &mut miniq,
        "q",
        json!({"RetentionSeconds": 60, "VisibilityTimeout": 1000}),
    );
    send(&mut miniq, "q", "in flight");
    send(&mut miniq, "q", "visible");
    miniq.ok(
        json!({"Command": "SendMessage", "QueueName": "q", "Body": "delayed", "DelaySeconds": 900}),
    );
    miniq.ok(json!({"Command": "ReceiveMessage", "QueueName": "q"}));
    assert_eq!(
        counts(&mut miniq, "q"),
        json!({"Visible": 1, "InFlight": 1, "Delayed": 1})
    );
    tick(&mut miniq, 60);
    assert_eq!(
        counts(&mut miniq, "q"),
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn expired_messages_do_not_count_toward_the_queue_limit() {
    let mut miniq = Miniq::new();
    create(&mut miniq, "q", json!({"RetentionSeconds": 60}));
    for _ in 0..100_000 {
        let response = miniq.call(json!({"Command": "SendMessage", "QueueName": "q", "Body": "x"}));
        assert_eq!(response["ok"], true);
    }
    tick(&mut miniq, 60);
    send(&mut miniq, "q", "fresh");
    assert_eq!(total(&mut miniq, "q"), 1);
}

#[test]
fn lowering_retention_removes_at_once_and_raising_it_brings_nothing_back() {
    let mut miniq = Miniq::new();
    create(&mut miniq, "q", json!({}));
    send(&mut miniq, "q", "old");
    tick(&mut miniq, 100);
    send(&mut miniq, "q", "new");
    set(&mut miniq, "q", json!({"RetentionSeconds": 100}));
    assert_eq!(total(&mut miniq, "q"), 1);
    set(&mut miniq, "q", json!({"RetentionSeconds": 1209600}));
    assert_eq!(total(&mut miniq, "q"), 1);
    let received = miniq.ok(json!({"Command": "ReceiveMessage", "QueueName": "q"}));
    assert_eq!(received["Messages"][0]["Body"], "new");
}

#[test]
fn messages_that_expired_before_a_retention_increase_stay_gone() {
    let mut miniq = Miniq::new();
    create(&mut miniq, "q", json!({"RetentionSeconds": 60}));
    send(&mut miniq, "q", "a");
    tick(&mut miniq, 61);
    // No command touched the queue while "a" expired; raising retention must
    // not resurrect it.
    set(&mut miniq, "q", json!({"RetentionSeconds": 1000}));
    assert_eq!(total(&mut miniq, "q"), 0);
}

#[test]
fn retention_does_not_free_deduplication_ids() {
    let mut miniq = Miniq::new();
    create(
        &mut miniq,
        "q.fifo",
        json!({"Fifo": true, "RetentionSeconds": 60}),
    );
    let send_dup = |miniq: &mut Miniq| {
        miniq.ok(json!({"Command": "SendMessage", "QueueName": "q.fifo", "Body": "x", "GroupId": "g", "DeduplicationId": "d"}))
    };
    send_dup(&mut miniq);
    tick(&mut miniq, 60);
    assert_eq!(total(&mut miniq, "q.fifo"), 0);
    assert_eq!(send_dup(&mut miniq)["MessageId"], "m-1");
    assert_eq!(total(&mut miniq, "q.fifo"), 0);
}

#[test]
fn moved_messages_expire_by_their_original_send_time() {
    let mut miniq = Miniq::new();
    create(&mut miniq, "dead", json!({"RetentionSeconds": 100}));
    create(
        &mut miniq,
        "q",
        json!({"VisibilityTimeout": 0, "DeadLetterQueue": "dead", "MaxReceiveCount": 1}),
    );
    send(&mut miniq, "q", "old");
    tick(&mut miniq, 50);
    send(&mut miniq, "q", "newer");
    // Receive "old", then move it; the dead-letter queue sees it with SentAt 0.
    miniq.ok(json!({"Command": "ReceiveMessage", "QueueName": "q"}));
    miniq.ok(json!({"Command": "ReceiveMessage", "QueueName": "q"}));
    send(&mut miniq, "dead", "direct");
    assert_eq!(counts(&mut miniq, "dead")["Visible"], 2);
    // "old" (SentAt 0) goes at 100, though it arrived after nothing older.
    tick(&mut miniq, 50);
    let remaining =
        miniq.ok(json!({"Command": "ReceiveMessage", "QueueName": "dead", "MaxMessages": 10}));
    let bodies: Vec<&str> = remaining["Messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["Body"].as_str().unwrap())
        .collect();
    assert_eq!(bodies, ["direct"]);
}
