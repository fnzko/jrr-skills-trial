mod common;

use common::Harness;
use serde_json::{Value, json};

fn create(h: &mut Harness, name: &str, attributes: Value) {
    h.ok(json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attributes}));
}

fn send(h: &mut Harness, queue: &str, body: &str) -> Value {
    h.ok(json!({"Command": "SendMessage", "QueueName": queue, "Body": body}))
}

fn send_fifo(h: &mut Harness, queue: &str, body: &str, group: &str, dedup: &str) -> Value {
    h.ok(
        json!({"Command": "SendMessage", "QueueName": queue, "Body": body,
                "GroupId": group, "DeduplicationId": dedup}),
    )
}

fn receive(h: &mut Harness, queue: &str, max: u64) -> Vec<Value> {
    let r = h.ok(json!({"Command": "ReceiveMessage", "QueueName": queue, "MaxMessages": max}));
    r["Messages"].as_array().unwrap().clone()
}

fn bodies(messages: &[Value]) -> Vec<&str> {
    messages
        .iter()
        .map(|m| m["Body"].as_str().unwrap())
        .collect()
}

fn counts(h: &mut Harness, queue: &str) -> Value {
    h.ok(json!({"Command": "GetQueueAttributes", "QueueName": queue}))["Counts"].clone()
}

fn fifo() -> Value {
    json!({"Fifo": true})
}

#[test]
fn send_and_receive_round_trip() {
    let mut h = Harness::new();
    create(&mut h, "a", json!({}));
    create(&mut h, "b", json!({}));
    assert_eq!(
        send(&mut h, "a", "one"),
        json!({"ok": true, "MessageId": "m-1"})
    );
    h.tick(7);
    assert_eq!(send(&mut h, "b", "two")["MessageId"], "m-2");
    let got = receive(&mut h, "a", 1);
    assert_eq!(
        got,
        vec![
            json!({"MessageId": "m-1", "ReceiptHandle": "r-1", "Body": "one",
                    "ReceiveCount": 1, "SentAt": 0, "FirstReceivedAt": 7})
        ]
    );
    // Handles and ids count over all queues.
    assert_eq!(receive(&mut h, "b", 1)[0]["ReceiptHandle"], "r-2");
    assert_eq!(
        counts(&mut h, "a"),
        json!({"Visible": 0, "InFlight": 1, "Delayed": 0})
    );
}

#[test]
fn send_validation_order() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    create(&mut h, "f.fifo", fifo());
    let s = |q: &str, extra: Value| {
        let mut c = json!({"Command": "SendMessage", "QueueName": q, "Body": "x"});
        for (k, v) in extra.as_object().unwrap() {
            c[k] = v.clone();
        }
        c
    };
    assert_eq!(
        h.err(json!({"Command": "SendMessage", "QueueName": "nope"})),
        "MissingParameter"
    );
    assert_eq!(
        h.err(json!({"Command": "SendMessage", "Body": 1})),
        "MissingParameter"
    );
    assert_eq!(
        h.err(s("nope", json!({"Body": ""}))),
        "InvalidParameterValue"
    );
    assert_eq!(h.err(s("nope", json!({}))), "QueueDoesNotExist");
    assert_eq!(
        h.err(s("q", json!({"Body": "é".repeat(513)}))),
        "InvalidParameterValue"
    );
    h.ok(s("q", json!({"Body": "é".repeat(512)})));
    assert_eq!(
        h.err(s("q", json!({"DelaySeconds": 901}))),
        "InvalidParameterValue"
    );
    assert_eq!(
        h.err(s("q", json!({"DelaySeconds": 1.0}))),
        "InvalidParameterValue"
    );
    h.ok(s("q", json!({"DelaySeconds": 900})));
    assert_eq!(
        h.err(s("q", json!({"GroupId": "bad id"}))),
        "InvalidParameterValue"
    );
    assert_eq!(
        h.err(s("q", json!({"GroupId": ""}))),
        "InvalidParameterValue"
    );
    assert_eq!(
        h.err(s("q", json!({"DeduplicationId": "x".repeat(129)}))),
        "InvalidParameterValue"
    );
    assert_eq!(
        h.err(s("q", json!({"GroupId": "g"}))),
        "UnsupportedOperation"
    );
    assert_eq!(
        h.err(s("q", json!({"DeduplicationId": "d"}))),
        "UnsupportedOperation"
    );
    assert_eq!(
        h.err(s("f.fifo", json!({"DelaySeconds": 0, "GroupId": "g"}))),
        "UnsupportedOperation"
    );
    assert_eq!(
        h.err(s("f.fifo", json!({"DeduplicationId": "d"}))),
        "MissingParameter"
    );
    assert_eq!(
        h.err(s("f.fifo", json!({"GroupId": "g"}))),
        "MissingParameter"
    );
    // A rejected send does not use up a message id.
    assert_eq!(
        send_fifo(&mut h, "f.fifo", "x", "g", "d")["MessageId"],
        "m-3"
    );
}

#[test]
fn delay_hides_messages_until_due() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({"DelaySeconds": 10}));
    send(&mut h, "q", "queue delay");
    h.ok(json!({"Command": "SendMessage", "QueueName": "q", "Body": "own", "DelaySeconds": 0}));
    assert_eq!(
        counts(&mut h, "q"),
        json!({"Visible": 1, "InFlight": 0, "Delayed": 1})
    );
    assert_eq!(bodies(&receive(&mut h, "q", 10)), ["own"]);
    h.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"DelaySeconds": 0}}));
    h.tick(9);
    assert!(receive(&mut h, "q", 10).is_empty());
    h.tick(1);
    assert_eq!(bodies(&receive(&mut h, "q", 10)), ["queue delay"]);
}

#[test]
fn visibility_timeout_and_arrival_order() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({"VisibilityTimeout": 10}));
    for b in ["a", "b", "c"] {
        send(&mut h, "q", b);
    }
    assert_eq!(bodies(&receive(&mut h, "q", 1)), ["a"]);
    assert_eq!(bodies(&receive(&mut h, "q", 5)), ["b", "c"]);
    h.tick(9);
    assert!(receive(&mut h, "q", 5).is_empty());
    h.tick(1);
    let again = receive(&mut h, "q", 5);
    assert_eq!(bodies(&again), ["a", "b", "c"]);
    assert_eq!(again[0]["ReceiveCount"], 2);
    assert_eq!(again[0]["FirstReceivedAt"], 0);
    assert_eq!(again[0]["ReceiptHandle"], "r-4");
}

#[test]
fn receive_parameter_validation() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    for max in [json!(0), json!(11), json!("1"), json!(1.5)] {
        assert_eq!(
            h.err(json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": max})),
            "InvalidParameterValue"
        );
    }
    assert_eq!(
        h.err(json!({"Command": "ReceiveMessage", "QueueName": "q", "VisibilityTimeout": 43201})),
        "InvalidParameterValue"
    );
    assert_eq!(
        h.err(json!({"Command": "ReceiveMessage"})),
        "MissingParameter"
    );
}

#[test]
fn zero_visibility_makes_handle_unusable() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    send(&mut h, "q", "x");
    let r = h.ok(json!({"Command": "ReceiveMessage", "QueueName": "q", "VisibilityTimeout": 0}));
    assert_eq!(r["Messages"][0]["ReceiptHandle"], "r-1");
    assert_eq!(
        h.err(json!({"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": "r-1"})),
        "ReceiptHandleExpired"
    );
    assert_eq!(receive(&mut h, "q", 1)[0]["ReceiptHandle"], "r-2");
}

#[test]
fn delete_and_change_visibility_handle_errors() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    create(&mut h, "other", json!({}));
    send(&mut h, "q", "x");
    send(&mut h, "other", "y");
    receive(&mut h, "q", 1); // r-1
    receive(&mut h, "other", 1); // r-2
    let del = |q: &str, handle: &str| json!({"Command": "DeleteMessage", "QueueName": q, "ReceiptHandle": handle});
    for bad in [
        "r-01",
        "R-1",
        "r-1 ",
        "r-3",
        "r-",
        "",
        "r--1",
        "r-+1",
        "1",
        "r-99999999999999999999999",
    ] {
        assert_eq!(h.err(del("q", bad)), "ReceiptHandleInvalid", "{bad:?}");
    }
    assert_eq!(h.err(del("q", "r-2")), "ReceiptHandleExpired");
    assert_eq!(h.err(del("nope", "r-1")), "QueueDoesNotExist");
    assert_eq!(
        h.err(json!({"Command": "DeleteMessage", "QueueName": "q"})),
        "MissingParameter"
    );
    assert_eq!(
        h.err(json!({"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": 1})),
        "InvalidParameterValue"
    );

    h.tick(30);
    assert_eq!(h.err(del("q", "r-1")), "ReceiptHandleExpired");
    assert_eq!(receive(&mut h, "q", 1)[0]["ReceiptHandle"], "r-3");
    assert_eq!(h.err(del("q", "r-1")), "ReceiptHandleExpired");
    h.ok(del("q", "r-3"));
    assert_eq!(h.err(del("q", "r-3")), "ReceiptHandleExpired");
    assert_eq!(
        counts(&mut h, "q"),
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn change_message_visibility_resets_the_deadline() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({"VisibilityTimeout": 10}));
    send(&mut h, "q", "x");
    receive(&mut h, "q", 1);
    let change = |t: Value| {
        json!({"Command": "ChangeMessageVisibility", "QueueName": "q",
                                   "ReceiptHandle": "r-1", "VisibilityTimeout": t})
    };
    assert_eq!(
        h.err(
            json!({"Command": "ChangeMessageVisibility", "QueueName": "q", "ReceiptHandle": "r-1"})
        ),
        "MissingParameter"
    );
    assert_eq!(h.err(change(json!(43201))), "InvalidParameterValue");
    h.tick(8);
    h.ok(change(json!(5)));
    h.tick(4);
    assert!(receive(&mut h, "q", 1).is_empty());
    h.tick(1);
    let again = receive(&mut h, "q", 1);
    assert_eq!(again[0]["ReceiveCount"], 2);
    // 0 makes the message visible at once.
    h.ok(
        json!({"Command": "ChangeMessageVisibility", "QueueName": "q",
                "ReceiptHandle": "r-2", "VisibilityTimeout": 0}),
    );
    assert_eq!(receive(&mut h, "q", 1).len(), 1);
    assert_eq!(
        h.err(
            json!({"Command": "ChangeMessageVisibility", "QueueName": "q",
                     "ReceiptHandle": "r-2", "VisibilityTimeout": 5})
        ),
        "ReceiptHandleExpired"
    );
}

#[test]
fn retention_removes_messages_whatever_their_state() {
    let mut h = Harness::new();
    create(
        &mut h,
        "q",
        json!({"RetentionSeconds": 100, "DelaySeconds": 500, "VisibilityTimeout": 1000}),
    );
    h.ok(json!({"Command": "SendMessage", "QueueName": "q", "Body": "delayed"}));
    h.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"DelaySeconds": 0}}));
    send(&mut h, "q", "flying");
    receive(&mut h, "q", 1);
    h.tick(99);
    assert_eq!(
        counts(&mut h, "q"),
        json!({"Visible": 0, "InFlight": 1, "Delayed": 1})
    );
    h.tick(1);
    assert_eq!(
        counts(&mut h, "q"),
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(
        h.err(json!({"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": "r-1"})),
        "ReceiptHandleExpired"
    );
}

#[test]
fn changing_retention_applies_to_existing_messages_at_once() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    send(&mut h, "q", "old");
    h.tick(60);
    send(&mut h, "q", "new");
    let set = |r: u64| {
        json!({"Command": "SetQueueAttributes", "QueueName": "q",
                              "Attributes": {"RetentionSeconds": r}})
    };
    h.ok(set(60));
    assert_eq!(bodies(&receive(&mut h, "q", 10)), ["new"]);
    h.ok(set(1_209_600));
    assert_eq!(counts(&mut h, "q")["InFlight"], 1);
    assert_eq!(counts(&mut h, "q")["Visible"], 0);
}

#[test]
fn purge_removes_everything_but_keeps_sequence_and_dedup() {
    let mut h = Harness::new();
    create(&mut h, "f.fifo", fifo());
    assert_eq!(
        send_fifo(&mut h, "f.fifo", "a", "g", "d1")["SequenceNumber"],
        1
    );
    receive(&mut h, "f.fifo", 1);
    h.ok(json!({"Command": "PurgeQueue", "QueueName": "f.fifo"}));
    assert_eq!(
        counts(&mut h, "f.fifo"),
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(
        h.err(json!({"Command": "DeleteMessage", "QueueName": "f.fifo", "ReceiptHandle": "r-1"})),
        "ReceiptHandleExpired"
    );
    let dup = send_fifo(&mut h, "f.fifo", "a", "g", "d1");
    assert_eq!(
        dup,
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
    assert_eq!(counts(&mut h, "f.fifo")["Visible"], 0);
    assert_eq!(
        send_fifo(&mut h, "f.fifo", "b", "g", "d2")["SequenceNumber"],
        2
    );
}

#[test]
fn fifo_delivers_one_message_per_group_in_order() {
    let mut h = Harness::new();
    create(&mut h, "f.fifo", fifo());
    send_fifo(&mut h, "f.fifo", "a1", "A", "1");
    send_fifo(&mut h, "f.fifo", "b1", "B", "2");
    send_fifo(&mut h, "f.fifo", "a2", "A", "3");
    send_fifo(&mut h, "f.fifo", "c1", "C", "4");
    let first = receive(&mut h, "f.fifo", 10);
    assert_eq!(bodies(&first), ["a1", "b1", "c1"]);
    assert_eq!(first[0]["GroupId"], "A");
    assert_eq!(first[1]["SequenceNumber"], 2);
    // Groups with an in-flight head stay blocked.
    assert!(receive(&mut h, "f.fifo", 10).is_empty());
    h.ok(json!({"Command": "DeleteMessage", "QueueName": "f.fifo", "ReceiptHandle": "r-1"}));
    assert_eq!(bodies(&receive(&mut h, "f.fifo", 10)), ["a2"]);
}

#[test]
fn fifo_group_head_in_flight_blocks_later_messages_even_after_timeouts_differ() {
    let mut h = Harness::new();
    create(
        &mut h,
        "f.fifo",
        json!({"Fifo": true, "VisibilityTimeout": 5}),
    );
    send_fifo(&mut h, "f.fifo", "a1", "A", "1");
    send_fifo(&mut h, "f.fifo", "a2", "A", "2");
    receive(&mut h, "f.fifo", 1);
    h.tick(5);
    // a1 is visible again and goes first; a2 is never ahead of it.
    assert_eq!(bodies(&receive(&mut h, "f.fifo", 10)), ["a1"]);
}

#[test]
fn deduplication_window() {
    let mut h = Harness::new();
    create(&mut h, "f.fifo", fifo());
    assert_eq!(
        send_fifo(&mut h, "f.fifo", "x", "A", "d")["MessageId"],
        "m-1"
    );
    h.tick(299);
    // Duplicates ignore GroupId and body and do not extend the window.
    let dup = send_fifo(&mut h, "f.fifo", "other body", "B", "d");
    assert_eq!(
        dup,
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
    h.tick(1);
    let fresh = send_fifo(&mut h, "f.fifo", "x2", "A", "d");
    assert_eq!(
        fresh,
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2})
    );
    h.tick(299);
    assert_eq!(
        send_fifo(&mut h, "f.fifo", "x3", "A", "d")["MessageId"],
        "m-2"
    );
}

#[test]
fn deduplication_outlives_the_message() {
    let mut h = Harness::new();
    create(&mut h, "f.fifo", fifo());
    send_fifo(&mut h, "f.fifo", "x", "A", "d");
    receive(&mut h, "f.fifo", 1);
    h.ok(json!({"Command": "DeleteMessage", "QueueName": "f.fifo", "ReceiptHandle": "r-1"}));
    assert_eq!(
        send_fifo(&mut h, "f.fifo", "x", "A", "d")["MessageId"],
        "m-1"
    );
    assert_eq!(counts(&mut h, "f.fifo")["Visible"], 0);
}

#[test]
fn content_based_deduplication_uses_the_body() {
    let mut h = Harness::new();
    create(
        &mut h,
        "f.fifo",
        json!({"Fifo": true, "ContentBasedDeduplication": true}),
    );
    let send = |h: &mut Harness, body: &str| {
        h.ok(json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": body, "GroupId": "g"}))
    };
    assert_eq!(send(&mut h, "hello")["MessageId"], "m-1");
    assert_eq!(send(&mut h, "hello")["MessageId"], "m-1");
    assert_eq!(send(&mut h, "world")["MessageId"], "m-2");
    // An explicit id wins over the body.
    assert_eq!(
        send_fifo(&mut h, "f.fifo", "hello", "g", "explicit")["MessageId"],
        "m-3"
    );
    // Deduplication ids are per queue.
    create(&mut h, "g.fifo", fifo());
    assert_eq!(
        send_fifo(&mut h, "g.fifo", "hello", "g", "explicit")["MessageId"],
        "m-4"
    );
}

#[test]
fn dead_letter_move_resets_the_message() {
    let mut h = Harness::new();
    create(&mut h, "dlq", json!({"VisibilityTimeout": 100}));
    create(
        &mut h,
        "q",
        json!({"VisibilityTimeout": 1, "DeadLetterQueue": "dlq", "MaxReceiveCount": 2}),
    );
    send(&mut h, "dlq", "resident");
    send(&mut h, "q", "poison");
    send(&mut h, "q", "fine");
    receive(&mut h, "q", 1);
    h.tick(1);
    assert_eq!(bodies(&receive(&mut h, "q", 1)), ["poison"]);
    h.tick(1);
    // poison has been received twice: it moves and does not count toward the limit.
    assert_eq!(bodies(&receive(&mut h, "q", 1)), ["fine"]);
    assert_eq!(
        counts(&mut h, "dlq"),
        json!({"Visible": 2, "InFlight": 0, "Delayed": 0})
    );
    let moved = receive(&mut h, "dlq", 10);
    assert_eq!(bodies(&moved), ["resident", "poison"]);
    assert_eq!(moved[1]["MessageId"], "m-2");
    assert_eq!(moved[1]["SentAt"], 0);
    assert_eq!(moved[1]["ReceiveCount"], 1);
    assert_eq!(moved[1]["FirstReceivedAt"], 2);
    // Handles of moved messages are expired.
    assert_eq!(
        h.err(json!({"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": "r-2"})),
        "ReceiptHandleExpired"
    );
}

#[test]
fn dead_letter_move_in_fifo_queues() {
    let mut h = Harness::new();
    create(&mut h, "d.fifo", fifo());
    create(
        &mut h,
        "q.fifo",
        json!({"Fifo": true, "VisibilityTimeout": 0,
                                    "DeadLetterQueue": "d.fifo", "MaxReceiveCount": 1}),
    );
    send_fifo(&mut h, "d.fifo", "resident", "g", "r");
    send_fifo(&mut h, "q.fifo", "a1", "A", "1");
    send_fifo(&mut h, "q.fifo", "a2", "A", "2");
    assert_eq!(bodies(&receive(&mut h, "q.fifo", 5)), ["a1"]);
    // a1 moves away, which frees its group: a2 is delivered in the same call.
    assert_eq!(bodies(&receive(&mut h, "q.fifo", 5)), ["a2"]);
    let moved = receive(&mut h, "d.fifo", 5);
    assert_eq!(bodies(&moved), ["resident", "a1"]);
    assert_eq!(moved[1]["GroupId"], "A");
    assert_eq!(moved[1]["SequenceNumber"], 2);
}

#[test]
fn dead_letter_arrival_respects_destination_retention_and_ignores_limits() {
    let mut h = Harness::new();
    create(&mut h, "dlq", json!({"RetentionSeconds": 60}));
    create(
        &mut h,
        "q",
        json!({"VisibilityTimeout": 0, "DeadLetterQueue": "dlq", "MaxReceiveCount": 1}),
    );
    send(&mut h, "q", "old");
    receive(&mut h, "q", 1);
    h.tick(60);
    assert!(receive(&mut h, "q", 1).is_empty());
    assert_eq!(
        counts(&mut h, "dlq"),
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(
        counts(&mut h, "q"),
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn dead_letter_configuration_can_be_replaced() {
    let mut h = Harness::new();
    create(&mut h, "d1", json!({}));
    create(&mut h, "d2", json!({}));
    create(
        &mut h,
        "q",
        json!({"VisibilityTimeout": 0, "DeadLetterQueue": "d1", "MaxReceiveCount": 5}),
    );
    send(&mut h, "q", "x");
    receive(&mut h, "q", 1);
    h.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q",
                "Attributes": {"DeadLetterQueue": "d2", "MaxReceiveCount": 1}}));
    assert!(receive(&mut h, "q", 1).is_empty());
    assert_eq!(counts(&mut h, "d2")["Visible"], 1);
    assert_eq!(counts(&mut h, "d1")["Visible"], 0);
}

#[test]
fn a_full_queue_rejects_sends_quickly() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let started = std::time::Instant::now();
    for _ in 0..100_000 {
        h.ok(json!({"Command": "SendMessage", "QueueName": "q", "Body": "x"}));
    }
    assert_eq!(
        h.err(json!({"Command": "SendMessage", "QueueName": "q", "Body": "x"})),
        "LimitExceeded"
    );
    assert!(
        started.elapsed().as_secs() < 20,
        "filling a queue is too slow"
    );
    // A rejected send used no id.
    h.ok(json!({"Command": "PurgeQueue", "QueueName": "q"}));
    assert_eq!(send(&mut h, "q", "y")["MessageId"], "m-100001");
}

#[test]
fn max_message_bytes_limits_body_in_utf8_bytes() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({"MaxMessageBytes": 4}));
    send(&mut h, "q", "abcd");
    // "é" is 2 bytes in UTF-8, so three of them are 6 bytes.
    for body in ["abcde", "ééé"] {
        assert_eq!(
            h.err(json!({"Command": "SendMessage", "QueueName": "q", "Body": body})),
            "InvalidParameterValue"
        );
    }
    send(&mut h, "q", "éé");
    assert_eq!(counts(&mut h, "q")["Visible"], 2);
}

#[test]
fn max_message_bytes_check_precedes_send_rules_and_failed_send_uses_no_id() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({"MaxMessageBytes": 2}));
    // Rule 0 beats rule 1 (UnsupportedOperation for GroupId on a standard queue).
    assert_eq!(
        h.err(json!({"Command": "SendMessage", "QueueName": "q", "Body": "abc", "GroupId": "g"})),
        "InvalidParameterValue"
    );
    assert_eq!(send(&mut h, "q", "ab")["MessageId"], "m-1");
}

#[test]
fn max_message_bytes_default_range_and_change() {
    let mut h = Harness::new();
    create(&mut h, "q", json!({}));
    let attrs =
        h.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}))["Attributes"].clone();
    assert_eq!(attrs["MaxMessageBytes"], 1024);
    for bad in [json!(0), json!(1025), json!("5"), json!(5.0)] {
        assert_eq!(
            h.err(json!({"Command": "SetQueueAttributes", "QueueName": "q",
                         "Attributes": {"MaxMessageBytes": bad}})),
            "InvalidParameterValue"
        );
    }
    send(&mut h, "q", "abcdef");
    h.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q",
                "Attributes": {"MaxMessageBytes": 3}}));
    // Existing messages stay; new ones are limited.
    assert_eq!(counts(&mut h, "q")["Visible"], 1);
    assert_eq!(
        h.err(json!({"Command": "SendMessage", "QueueName": "q", "Body": "abcd"})),
        "InvalidParameterValue"
    );
    // CreateQueue rule 4 compares it.
    assert_eq!(
        h.err(json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": {}})),
        "QueueAlreadyExists"
    );
    create(&mut h, "q", json!({"MaxMessageBytes": 3}));
}

#[test]
fn dead_letter_move_ignores_max_message_bytes() {
    let mut h = Harness::new();
    create(&mut h, "dlq", json!({"MaxMessageBytes": 1}));
    create(
        &mut h,
        "q",
        json!({"DeadLetterQueue": "dlq", "MaxReceiveCount": 1, "VisibilityTimeout": 0}),
    );
    send(&mut h, "q", "long body");
    assert_eq!(receive(&mut h, "q", 1).len(), 1);
    assert_eq!(receive(&mut h, "q", 1).len(), 0);
    assert_eq!(bodies(&receive(&mut h, "dlq", 1)), ["long body"]);
}
