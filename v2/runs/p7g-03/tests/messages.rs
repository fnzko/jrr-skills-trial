mod common;

use common::Session;
use serde_json::{Value, json};

fn send(s: &mut Session, queue: &str, body: &str) -> Value {
    s.ok(json!({"Command": "SendMessage", "QueueName": queue, "Body": body}))
}

fn send_fifo(s: &mut Session, queue: &str, body: &str, group: &str, dedup: &str) -> Value {
    s.ok(
        json!({"Command": "SendMessage", "QueueName": queue, "Body": body,
                "GroupId": group, "DeduplicationId": dedup}),
    )
}

fn receive(s: &mut Session, queue: &str, max: u32) -> Vec<Value> {
    let r = s.ok(json!({"Command": "ReceiveMessage", "QueueName": queue, "MaxMessages": max}));
    r["Messages"].as_array().unwrap().clone()
}

fn bodies(messages: &[Value]) -> Vec<&str> {
    messages
        .iter()
        .map(|m| m["Body"].as_str().unwrap())
        .collect()
}

#[test]
fn ids_count_accepted_messages_over_all_queues() {
    let mut s = Session::new();
    s.create("a", json!({}));
    s.create("b", json!({}));
    assert_eq!(send(&mut s, "a", "x")["MessageId"], "m-1");
    assert_eq!(send(&mut s, "b", "x")["MessageId"], "m-2");
    assert_eq!(
        s.err(json!({"Command": "SendMessage", "QueueName": "a", "Body": ""})),
        "InvalidParameterValue"
    );
    assert_eq!(send(&mut s, "a", "x")["MessageId"], "m-3");
}

#[test]
fn send_validation_and_order_of_checks() {
    let mut s = Session::new();
    s.create("q", json!({}));
    s.create("f.fifo", json!({"Fifo": true}));
    let c = |v: Value| {
        let mut base = json!({"Command": "SendMessage", "QueueName": "q", "Body": "b"});
        base.as_object_mut()
            .unwrap()
            .extend(v.as_object().unwrap().clone());
        base
    };
    assert_eq!(
        s.err(json!({"Command": "SendMessage", "QueueName": "q"})),
        "MissingParameter"
    );
    assert_eq!(
        s.err(json!({"Command": "SendMessage", "QueueName": 1})),
        "MissingParameter"
    );
    assert_eq!(
        s.err(c(json!({"Body": "é".repeat(513)}))),
        "InvalidParameterValue"
    );
    s.ok(c(json!({"Body": "é".repeat(512)})));
    assert_eq!(
        s.err(c(json!({"DelaySeconds": 901}))),
        "InvalidParameterValue"
    );
    assert_eq!(
        s.err(c(json!({"GroupId": "bad id"}))),
        "InvalidParameterValue"
    );
    assert_eq!(s.err(c(json!({"GroupId": "g"}))), "UnsupportedOperation");
    assert_eq!(
        s.err(c(json!({"DeduplicationId": "d"}))),
        "UnsupportedOperation"
    );
    assert_eq!(s.err(c(json!({"QueueName": "zzz"}))), "QueueDoesNotExist");
    // Value errors outrank the queue lookup.
    assert_eq!(
        s.err(c(json!({"QueueName": "zzz", "DelaySeconds": -1}))),
        "InvalidParameterValue"
    );
    let f = |v: Value| {
        let mut base = json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "b"});
        base.as_object_mut()
            .unwrap()
            .extend(v.as_object().unwrap().clone());
        base
    };
    assert_eq!(
        s.err(f(json!({"DelaySeconds": 1, "GroupId": "g"}))),
        "UnsupportedOperation"
    );
    assert_eq!(s.err(f(json!({}))), "MissingParameter");
    assert_eq!(s.err(f(json!({"GroupId": "g"}))), "MissingParameter");
    assert_eq!(
        s.ok(f(json!({"GroupId": "g", "DeduplicationId": "d"})))["SequenceNumber"],
        1
    );
}

#[test]
fn receive_delivers_in_arrival_order_and_hides_messages() {
    let mut s = Session::new();
    s.create("q", json!({"VisibilityTimeout": 10}));
    for b in ["a", "b", "c"] {
        send(&mut s, "q", b);
    }
    s.tick(4);
    let first = receive(&mut s, "q", 2);
    assert_eq!(bodies(&first), ["a", "b"]);
    assert_eq!(first[0]["ReceiptHandle"], "r-1");
    assert_eq!(first[1]["ReceiptHandle"], "r-2");
    assert_eq!(first[0]["ReceiveCount"], 1);
    assert_eq!(first[0]["SentAt"], 0);
    assert_eq!(first[0]["FirstReceivedAt"], 4);
    assert!(first[0].get("GroupId").is_none());
    assert_eq!(
        s.counts("q"),
        json!({"Visible": 1, "InFlight": 2, "Delayed": 0})
    );
    assert_eq!(bodies(&receive(&mut s, "q", 10)), ["c"]);
    assert!(receive(&mut s, "q", 10).is_empty());
    s.tick(10);
    // Back in arrival order once the timeout ends.
    let again = receive(&mut s, "q", 10);
    assert_eq!(bodies(&again), ["a", "b", "c"]);
    assert_eq!(again[0]["ReceiveCount"], 2);
    assert_eq!(again[0]["FirstReceivedAt"], 4);
    assert_eq!(again[0]["ReceiptHandle"], "r-4");
}

#[test]
fn delay_hides_messages_until_due() {
    let mut s = Session::new();
    s.create("q", json!({"DelaySeconds": 5}));
    send(&mut s, "q", "queue-delay");
    s.ok(json!({"Command": "SendMessage", "QueueName": "q", "Body": "own", "DelaySeconds": 0}));
    assert_eq!(bodies(&receive(&mut s, "q", 10)), ["own"]);
    assert_eq!(s.counts("q")["Delayed"], 1);
    s.tick(4);
    assert!(receive(&mut s, "q", 10).is_empty());
    s.tick(1);
    assert_eq!(bodies(&receive(&mut s, "q", 10)), ["queue-delay"]);
}

#[test]
fn receive_parameter_checks() {
    let mut s = Session::new();
    s.create("q", json!({}));
    for bad in [
        json!({"MaxMessages": 0}),
        json!({"MaxMessages": 11}),
        json!({"VisibilityTimeout": 43201}),
    ] {
        let mut c = json!({"Command": "ReceiveMessage", "QueueName": "q"});
        c.as_object_mut()
            .unwrap()
            .extend(bad.as_object().unwrap().clone());
        assert_eq!(s.err(c), "InvalidParameterValue");
    }
    assert_eq!(
        s.err(json!({"Command": "ReceiveMessage"})),
        "MissingParameter"
    );
    assert_eq!(
        s.err(json!({"Command": "ReceiveMessage", "QueueName": "x"})),
        "QueueDoesNotExist"
    );
}

#[test]
fn delete_and_change_visibility() {
    let mut s = Session::new();
    s.create("q", json!({"VisibilityTimeout": 10}));
    s.create("other", json!({}));
    send(&mut s, "q", "a");
    let handle = receive(&mut s, "q", 1)[0]["ReceiptHandle"]
        .as_str()
        .unwrap()
        .to_owned();
    let cmd = |name: &str, h: &str| json!({"Command": "DeleteMessage", "QueueName": name, "ReceiptHandle": h});
    for never in ["r-0", "r-2", "r-01", "R-1", "r-1 ", "x", ""] {
        assert_eq!(s.err(cmd("q", never)), "ReceiptHandleInvalid", "{never:?}");
    }
    assert_eq!(s.err(cmd("other", &handle)), "ReceiptHandleExpired");
    assert_eq!(s.err(cmd("nope", &handle)), "QueueDoesNotExist");
    assert_eq!(
        s.err(json!({"Command": "DeleteMessage", "QueueName": "q"})),
        "MissingParameter"
    );

    // Extend, then shorten to nothing.
    let change = |t: i64| {
        json!({"Command": "ChangeMessageVisibility", "QueueName": "q",
                                 "ReceiptHandle": handle, "VisibilityTimeout": t})
    };
    s.tick(9);
    s.ok(change(5));
    s.tick(4);
    assert!(receive(&mut s, "q", 1).is_empty());
    assert_eq!(s.err(change(-1)), "InvalidParameterValue");
    s.ok(change(0));
    assert_eq!(s.err(change(5)), "ReceiptHandleExpired");
    let again = receive(&mut s, "q", 1);
    assert_eq!(again[0]["ReceiveCount"], 2);
    // The old handle is stale after redelivery; the new one deletes.
    assert_eq!(s.err(cmd("q", &handle)), "ReceiptHandleExpired");
    s.ok(cmd("q", again[0]["ReceiptHandle"].as_str().unwrap()));
    assert_eq!(
        s.err(cmd("q", again[0]["ReceiptHandle"].as_str().unwrap())),
        "ReceiptHandleExpired"
    );
    assert_eq!(
        s.counts("q"),
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn zero_visibility_timeout_makes_the_handle_unusable() {
    let mut s = Session::new();
    s.create("q", json!({}));
    send(&mut s, "q", "a");
    let r = s.ok(json!({"Command": "ReceiveMessage", "QueueName": "q", "VisibilityTimeout": 0}));
    let h = r["Messages"][0]["ReceiptHandle"].clone();
    assert_eq!(
        s.err(json!({"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": h})),
        "ReceiptHandleExpired"
    );
}

#[test]
fn handles_do_not_survive_purge_delete_or_recreate() {
    let mut s = Session::new();
    s.create("q", json!({}));
    send(&mut s, "q", "a");
    receive(&mut s, "q", 1);
    s.ok(json!({"Command": "PurgeQueue", "QueueName": "q"}));
    let del = json!({"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": "r-1"});
    assert_eq!(s.err(del.clone()), "ReceiptHandleExpired");
    send(&mut s, "q", "b");
    receive(&mut s, "q", 1);
    s.ok(json!({"Command": "DeleteQueue", "QueueName": "q"}));
    s.create("q", json!({}));
    send(&mut s, "q", "c");
    assert_eq!(s.err(del), "ReceiptHandleExpired");
    assert_eq!(
        s.err(json!({"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": "r-2"})),
        "ReceiptHandleExpired"
    );
}

#[test]
fn retention_removes_messages_in_every_state_and_is_not_undone() {
    let mut s = Session::new();
    s.create(
        "q",
        json!({"RetentionSeconds": 100, "DelaySeconds": 50, "VisibilityTimeout": 1000}),
    );
    send(&mut s, "q", "sent-at-0"); // delayed until 50
    s.tick(60);
    s.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"DelaySeconds": 0}}));
    send(&mut s, "q", "a");
    send(&mut s, "q", "b");
    assert_eq!(bodies(&receive(&mut s, "q", 1)), ["sent-at-0"]);
    assert_eq!(
        s.counts("q"),
        json!({"Visible": 2, "InFlight": 1, "Delayed": 0})
    );
    s.tick(39);
    assert_eq!(
        s.counts("q"),
        json!({"Visible": 2, "InFlight": 1, "Delayed": 0})
    );
    s.tick(1);
    // The in-flight message (sent at 0) expired at 100.
    assert_eq!(
        s.counts("q"),
        json!({"Visible": 2, "InFlight": 0, "Delayed": 0})
    );
    // Lowering retention removes messages at once: sent at 60, 60 + 60 = 120.
    s.tick(20);
    s.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"RetentionSeconds": 60}}));
    assert_eq!(
        s.counts("q"),
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
    s.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"RetentionSeconds": 1000}}));
    assert_eq!(
        s.counts("q"),
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn expiry_is_applied_before_a_retention_change_not_after() {
    let mut s = Session::new();
    s.create("q", json!({"RetentionSeconds": 60}));
    send(&mut s, "q", "old");
    s.tick(100);
    // Gone since t=60, even though nothing looked at the queue meanwhile.
    s.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"RetentionSeconds": 1000}}));
    assert_eq!(
        s.counts("q"),
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn queue_message_limit_counts_every_state() {
    let mut s = Session::new();
    s.create("q", json!({}));
    for _ in 0..100_000 {
        send(&mut s, "q", "x");
    }
    assert_eq!(
        s.err(json!({"Command": "SendMessage", "QueueName": "q", "Body": "x"})),
        "LimitExceeded"
    );
    s.ok(json!({"Command": "PurgeQueue", "QueueName": "q"}));
    send(&mut s, "q", "x");
}

#[test]
fn dead_letter_queue_receives_messages_after_max_receive_count() {
    let mut s = Session::new();
    s.create("dlq", json!({"RetentionSeconds": 100}));
    s.create(
        "q",
        json!({"VisibilityTimeout": 0, "DeadLetterQueue": "dlq", "MaxReceiveCount": 2}),
    );
    send(&mut s, "q", "poison");
    send(&mut s, "q", "fine");
    assert_eq!(bodies(&receive(&mut s, "q", 1)), ["poison"]);
    assert_eq!(bodies(&receive(&mut s, "q", 1)), ["poison"]);
    // Third look: moved, not delivered, and does not count toward MaxMessages.
    let third = receive(&mut s, "q", 1);
    assert_eq!(bodies(&third), ["fine"]);
    assert_eq!(
        s.counts("dlq"),
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
    let moved = receive(&mut s, "dlq", 1);
    assert_eq!(moved[0]["MessageId"], "m-1");
    assert_eq!(moved[0]["ReceiveCount"], 1);
    assert_eq!(moved[0]["SentAt"], 0);
}

#[test]
fn dead_lettered_message_older_than_dlq_retention_vanishes() {
    let mut s = Session::new();
    s.create("dlq", json!({"RetentionSeconds": 60}));
    s.create(
        "q",
        json!({"VisibilityTimeout": 0, "DeadLetterQueue": "dlq", "MaxReceiveCount": 1}),
    );
    send(&mut s, "q", "x");
    receive(&mut s, "q", 1);
    s.tick(60);
    assert!(receive(&mut s, "q", 1).is_empty());
    assert_eq!(
        s.counts("dlq"),
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn fifo_groups_deliver_one_at_a_time_in_order() {
    let mut s = Session::new();
    s.create("f.fifo", json!({"Fifo": true, "VisibilityTimeout": 10}));
    assert_eq!(
        send_fifo(&mut s, "f.fifo", "a1", "A", "1")["SequenceNumber"],
        1
    );
    send_fifo(&mut s, "f.fifo", "a2", "A", "2");
    send_fifo(&mut s, "f.fifo", "b1", "B", "3");
    let got = receive(&mut s, "f.fifo", 10);
    assert_eq!(bodies(&got), ["a1", "b1"]);
    assert_eq!(got[0]["GroupId"], "A");
    assert_eq!(got[0]["SequenceNumber"], 1);
    assert_eq!(got[1]["SequenceNumber"], 3);
    assert!(receive(&mut s, "f.fifo", 10).is_empty());
    s.ok(json!({"Command": "DeleteMessage", "QueueName": "f.fifo", "ReceiptHandle": got[0]["ReceiptHandle"]}));
    assert_eq!(bodies(&receive(&mut s, "f.fifo", 10)), ["a2"]);
}

#[test]
fn fifo_deduplication_window() {
    let mut s = Session::new();
    s.create("f.fifo", json!({"Fifo": true}));
    let first = send_fifo(&mut s, "f.fifo", "x", "g", "d");
    s.tick(299);
    let dup = send_fifo(&mut s, "f.fifo", "y", "other-group", "d");
    assert_eq!(dup["MessageId"], first["MessageId"]);
    assert_eq!(dup["SequenceNumber"], first["SequenceNumber"]);
    s.tick(1);
    // The duplicate did not extend the window; this opens a new one.
    let second = send_fifo(&mut s, "f.fifo", "z", "g", "d");
    assert_eq!(second["MessageId"], "m-2");
    assert_eq!(second["SequenceNumber"], 2);
    s.tick(299);
    assert_eq!(
        send_fifo(&mut s, "f.fifo", "z", "g", "d")["MessageId"],
        "m-2"
    );
}

#[test]
fn dedup_ids_outlive_messages_and_purge() {
    let mut s = Session::new();
    s.create(
        "f.fifo",
        json!({"Fifo": true, "ContentBasedDeduplication": true}),
    );
    let send_body = |s: &mut Session, b: &str| {
        s.ok(json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": b, "GroupId": "g"}))
    };
    assert_eq!(send_body(&mut s, "same")["MessageId"], "m-1");
    s.ok(json!({"Command": "PurgeQueue", "QueueName": "f.fifo"}));
    assert_eq!(send_body(&mut s, "same")["MessageId"], "m-1");
    // Sequence numbers carry on across a purge.
    assert_eq!(send_body(&mut s, "other")["SequenceNumber"], 2);
}

#[test]
fn fifo_dead_letter_resequences_and_unblocks_group() {
    let mut s = Session::new();
    s.create("d.fifo", json!({"Fifo": true}));
    s.create("q.fifo", json!({"Fifo": true, "VisibilityTimeout": 0, "DeadLetterQueue": "d.fifo", "MaxReceiveCount": 1}));
    send_fifo(&mut s, "q.fifo", "one", "g", "1");
    send_fifo(&mut s, "q.fifo", "two", "g", "2");
    assert_eq!(bodies(&receive(&mut s, "q.fifo", 5)), ["one"]);
    // "one" moves out, which frees the group for "two" in the same call.
    assert_eq!(bodies(&receive(&mut s, "q.fifo", 5)), ["two"]);
    let moved = receive(&mut s, "d.fifo", 1);
    assert_eq!(moved[0]["Body"], "one");
    assert_eq!(moved[0]["SequenceNumber"], 1);
    assert_eq!(moved[0]["GroupId"], "g");
}
