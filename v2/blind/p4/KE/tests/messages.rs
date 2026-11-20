//! Message commands, time, retention and dead-letter queues (SPEC sections 3, 4 and 6).

mod common;

use common::{Session, bodies, handle};
use serde_json::json;

#[test]
fn ids_count_across_queues_and_skip_failed_sends() {
    let mut s = Session::new();
    s.create("a", json!({}));
    s.create("b", json!({}));
    assert_eq!(s.send("a", "1"), "m-1");
    s.err(json!({"Command": "SendMessage", "QueueName": "a", "Body": "x", "GroupId": "g"}));
    assert_eq!(s.send("b", "2"), "m-2");

    let a = s.receive("a", 1);
    let b = s.receive("b", 1);
    assert_eq!(handle(&a[0]), "r-1");
    assert_eq!(handle(&b[0]), "r-2");
}

#[test]
fn received_message_shape() {
    let mut s = Session::new();
    s.create("q", json!({}));
    s.tick(3);
    s.send("q", "hello");
    s.tick(2);
    let messages = s.receive("q", 10);
    assert_eq!(
        messages,
        [json!({
            "MessageId": "m-1", "ReceiptHandle": "r-1", "Body": "hello",
            "ReceiveCount": 1, "SentAt": 3, "FirstReceivedAt": 5,
        })]
    );
}

#[test]
fn delays_hide_messages_until_they_end() {
    let mut s = Session::new();
    s.create("q", json!({"DelaySeconds": 10}));
    s.send("q", "queue-delay");
    s.ok(
        json!({"Command": "SendMessage", "QueueName": "q", "Body": "own-delay", "DelaySeconds": 0}),
    );
    assert_eq!(s.counts("q"), (1, 0, 1));
    assert_eq!(bodies(&s.receive("q", 10)), ["own-delay"]);
    s.tick(9);
    assert_eq!(s.receive("q", 10), Vec::<serde_json::Value>::new());
    s.tick(1);
    assert_eq!(bodies(&s.receive("q", 10)), ["queue-delay"]);
}

#[test]
fn visibility_timeout_returns_messages_in_arrival_order() {
    let mut s = Session::new();
    s.create("q", json!({"VisibilityTimeout": 10}));
    for body in ["a", "b", "c"] {
        s.send("q", body);
    }
    assert_eq!(bodies(&s.receive("q", 1)), ["a"]);
    assert_eq!(bodies(&s.receive("q", 1)), ["b"]);
    assert_eq!(s.counts("q"), (1, 2, 0));
    s.tick(10);
    let again = s.receive("q", 10);
    assert_eq!(bodies(&again), ["a", "b", "c"]);
    assert_eq!(again[0]["ReceiveCount"], 2);
    assert_eq!(again[0]["FirstReceivedAt"], 0);
}

#[test]
fn zero_visibility_timeout_never_yields_a_usable_handle() {
    let mut s = Session::new();
    s.create("q", json!({}));
    s.send("q", "a");
    let messages = s.receive_with_timeout("q", 10, 0);
    assert_eq!(messages.len(), 1);
    let delete = json!({"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": handle(&messages[0])});
    assert_eq!(s.err(delete), "ReceiptHandleExpired");
    assert_eq!(s.counts("q"), (1, 0, 0));
}

#[test]
fn receipt_handles_are_checked_exactly() {
    let mut s = Session::new();
    s.create("q", json!({}));
    s.create("other", json!({}));
    s.send("q", "a");
    let first = s.receive("q", 1);
    let delete = |queue: &str, handle: &str| json!({"Command": "DeleteMessage", "QueueName": queue, "ReceiptHandle": handle});

    for bogus in ["r-01", "R-1", "r-1 ", "r-2", "r-0", "r-", "r-+1", "1"] {
        assert_eq!(
            s.err(delete("q", bogus)),
            "ReceiptHandleInvalid",
            "{bogus:?}"
        );
    }
    assert_eq!(s.err(delete("other", "r-1")), "ReceiptHandleExpired");

    // Redelivery makes the old handle stale.
    s.tick(30);
    let second = s.receive("q", 1);
    assert_eq!(
        s.err(delete("q", handle(&first[0]))),
        "ReceiptHandleExpired"
    );
    s.ok(delete("q", handle(&second[0])));
    assert_eq!(
        s.err(delete("q", handle(&second[0]))),
        "ReceiptHandleExpired"
    );
    assert_eq!(s.counts("q"), (0, 0, 0));
}

#[test]
fn change_message_visibility_resets_the_timeout_from_now() {
    let mut s = Session::new();
    s.create("q", json!({}));
    s.send("q", "a");
    let message = s.receive("q", 1).remove(0);
    let change = |timeout: u64| {
        json!({
            "Command": "ChangeMessageVisibility", "QueueName": "q",
            "ReceiptHandle": handle(&message), "VisibilityTimeout": timeout,
        })
    };
    s.tick(20);
    s.ok(change(100));
    s.tick(99);
    assert_eq!(s.counts("q"), (0, 1, 0));
    s.ok(change(0));
    assert_eq!(s.counts("q"), (1, 0, 0));
    assert_eq!(s.err(change(10)), "ReceiptHandleExpired");
    let again = s.receive("q", 1);
    assert_eq!(again[0]["ReceiveCount"], 2);
}

#[test]
fn retention_removes_messages_whatever_their_state() {
    let mut s = Session::new();
    s.create("q", json!({"RetentionSeconds": 60}));
    s.send("q", "old");
    s.tick(30);
    s.send("q", "new");
    s.receive("q", 1);
    s.tick(30);
    assert_eq!(s.counts("q"), (1, 0, 0));
    assert_eq!(bodies(&s.receive("q", 10)), ["new"]);
}

#[test]
fn lowering_retention_removes_messages_at_once_for_good() {
    let mut s = Session::new();
    s.create("q", json!({}));
    s.send("q", "a");
    s.tick(100);
    let set = |seconds: u64| {
        json!({
            "Command": "SetQueueAttributes", "QueueName": "q",
            "Attributes": {"RetentionSeconds": seconds},
        })
    };
    s.ok(set(60));
    assert_eq!(s.counts("q"), (0, 0, 0));
    s.ok(set(1000));
    assert_eq!(s.counts("q"), (0, 0, 0));
}

#[test]
fn queue_attribute_changes_apply_to_later_operations_only() {
    let mut s = Session::new();
    s.create("q", json!({"DelaySeconds": 5}));
    s.send("q", "a");
    s.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"DelaySeconds": 0, "VisibilityTimeout": 1}}));
    assert_eq!(s.counts("q"), (0, 0, 1));
    s.tick(5);
    s.receive("q", 1);
    s.tick(1);
    assert_eq!(s.counts("q"), (1, 0, 0));
}

#[test]
fn send_rules_by_queue_type() {
    let mut s = Session::new();
    s.create("std", json!({}));
    s.create("q.fifo", json!({"Fifo": true}));
    let send = |queue: &str, extra: serde_json::Value| {
        let mut command = json!({"Command": "SendMessage", "QueueName": queue, "Body": "b"});
        command
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        command
    };
    assert_eq!(
        s.err(send("std", json!({"DeduplicationId": "d"}))),
        "UnsupportedOperation"
    );
    assert_eq!(
        s.err(send("q.fifo", json!({"DelaySeconds": 0, "GroupId": "g"}))),
        "UnsupportedOperation"
    );
    assert_eq!(
        s.err(send("q.fifo", json!({"DeduplicationId": "d"}))),
        "MissingParameter"
    );
    assert_eq!(
        s.err(send("q.fifo", json!({"GroupId": "g"}))),
        "MissingParameter"
    );
    assert_eq!(
        s.ok(send(
            "q.fifo",
            json!({"GroupId": "g", "DeduplicationId": "d"})
        )),
        json!({"MessageId": "m-1", "SequenceNumber": 1})
    );
}

#[test]
fn fifo_deduplication_window() {
    let mut s = Session::new();
    s.create(
        "q.fifo",
        json!({"Fifo": true, "ContentBasedDeduplication": true}),
    );
    let send = |body: &str, group: &str| json!({"Command": "SendMessage", "QueueName": "q.fifo", "Body": body, "GroupId": group});

    assert_eq!(
        s.ok(send("x", "g1")),
        json!({"MessageId": "m-1", "SequenceNumber": 1})
    );
    s.tick(299);
    // Duplicates ignore the group and do not extend the window.
    assert_eq!(
        s.ok(send("x", "g2")),
        json!({"MessageId": "m-1", "SequenceNumber": 1})
    );
    let message = s.receive("q.fifo", 1).remove(0);
    s.ok(json!({"Command": "DeleteMessage", "QueueName": "q.fifo", "ReceiptHandle": handle(&message)}));
    assert_eq!(
        s.ok(send("x", "g1")),
        json!({"MessageId": "m-1", "SequenceNumber": 1})
    );
    s.tick(1);
    assert_eq!(
        s.ok(send("x", "g1")),
        json!({"MessageId": "m-2", "SequenceNumber": 2})
    );
    // An explicit DeduplicationId takes precedence over the body.
    let explicit = json!({"Command": "SendMessage", "QueueName": "q.fifo", "Body": "x", "GroupId": "g", "DeduplicationId": "other"});
    assert_eq!(
        s.ok(explicit),
        json!({"MessageId": "m-3", "SequenceNumber": 3})
    );
}

#[test]
fn fifo_groups_deliver_in_order_one_at_a_time() {
    let mut s = Session::new();
    s.create("q.fifo", json!({"Fifo": true}));
    for (body, group) in [("a1", "a"), ("b1", "b"), ("a2", "a"), ("c1", "c")] {
        s.send_fifo("q.fifo", body, group);
    }
    let first = s.receive("q.fifo", 10);
    assert_eq!(bodies(&first), ["a1", "b1", "c1"]);
    assert_eq!(first[0]["GroupId"], "a");
    assert_eq!(first[0]["SequenceNumber"], 1);
    assert_eq!(s.receive("q.fifo", 10), Vec::<serde_json::Value>::new());

    s.ok(json!({"Command": "DeleteMessage", "QueueName": "q.fifo", "ReceiptHandle": handle(&first[0])}));
    assert_eq!(bodies(&s.receive("q.fifo", 10)), ["a2"]);
}

#[test]
fn messages_move_to_the_dead_letter_queue_after_max_receives() {
    let mut s = Session::new();
    s.create("dlq", json!({}));
    s.create("q", json!({"DeadLetterQueue": "dlq", "MaxReceiveCount": 2}));
    s.send("q", "poison");
    s.send("q", "fine");
    s.tick(5);
    for _ in 0..2 {
        assert_eq!(bodies(&s.receive_with_timeout("q", 1, 0)), ["poison"]);
    }
    // The poison message moves instead of being delivered and does not count toward MaxMessages.
    let messages = s.receive("q", 1);
    assert_eq!(bodies(&messages), ["fine"]);
    assert_eq!(s.counts("q"), (0, 1, 0));

    let moved = s.receive("dlq", 1);
    assert_eq!(moved[0]["MessageId"], "m-1");
    assert_eq!(moved[0]["SentAt"], 0);
    assert_eq!(moved[0]["ReceiveCount"], 1);
    assert_eq!(moved[0]["FirstReceivedAt"], 5);
}

#[test]
fn fifo_dead_letter_move_takes_a_new_sequence_number_and_unblocks_the_group() {
    let mut s = Session::new();
    s.create("dlq.fifo", json!({"Fifo": true}));
    s.create(
        "q.fifo",
        json!({"Fifo": true, "DeadLetterQueue": "dlq.fifo", "MaxReceiveCount": 1}),
    );
    s.send_fifo("dlq.fifo", "already-there", "z");
    s.send_fifo("q.fifo", "a1", "a");
    s.send_fifo("q.fifo", "a2", "a");
    s.receive_with_timeout("q.fifo", 1, 0);

    // a1 moves, so a2 is now first of its group and is delivered in the same receive.
    assert_eq!(bodies(&s.receive("q.fifo", 10)), ["a2"]);
    let moved = s.receive("dlq.fifo", 10);
    assert_eq!(bodies(&moved), ["already-there", "a1"]);
    assert_eq!(moved[1]["SequenceNumber"], 2);
    assert_eq!(moved[1]["GroupId"], "a");
}

#[test]
fn dead_letters_too_old_for_their_new_queue_vanish() {
    let mut s = Session::new();
    s.create("dlq", json!({"RetentionSeconds": 60}));
    s.create("q", json!({"DeadLetterQueue": "dlq", "MaxReceiveCount": 1}));
    s.send("q", "a");
    s.receive_with_timeout("q", 1, 0);
    s.tick(60);
    assert_eq!(s.receive("q", 10), Vec::<serde_json::Value>::new());
    assert_eq!(s.counts("q"), (0, 0, 0));
    assert_eq!(s.counts("dlq"), (0, 0, 0));
}

#[test]
fn moved_messages_invalidate_their_receipt_handles() {
    let mut s = Session::new();
    s.create("dlq", json!({}));
    s.create("q", json!({"DeadLetterQueue": "dlq", "MaxReceiveCount": 1}));
    s.send("q", "a");
    let message = s.receive_with_timeout("q", 1, 0).remove(0);
    s.receive("q", 1);
    let delete =
        json!({"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": handle(&message)});
    assert_eq!(s.err(delete), "ReceiptHandleExpired");
}

#[test]
fn a_queue_holds_at_most_100000_messages() {
    let mut s = Session::new();
    s.create("q", json!({}));
    for _ in 0..100_000 {
        s.send("q", "x");
    }
    assert_eq!(
        s.err(json!({"Command": "SendMessage", "QueueName": "q", "Body": "x"})),
        "LimitExceeded"
    );
    assert_eq!(s.counts("q"), (100_000, 0, 0));
    // Receiving deep into a full queue stays cheap because receives skip in-flight messages.
    for _ in 0..1_000 {
        assert_eq!(s.receive("q", 10).len(), 10);
    }
}
