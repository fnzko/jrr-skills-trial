mod common;

use common::Session;
use serde_json::{Value, json};

fn send(s: &mut Session, queue: &str, body: &str) -> Value {
    s.ok(json!({"Command": "SendMessage", "QueueName": queue, "Body": body}))
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

fn fifo_send(s: &mut Session, queue: &str, group: &str, dedup: &str, body: &str) -> Value {
    s.ok(
        json!({"Command": "SendMessage", "QueueName": queue, "Body": body,
                "GroupId": group, "DeduplicationId": dedup}),
    )
}

#[test]
fn ids_and_receive_shape_on_a_standard_queue() {
    let mut s = Session::new();
    s.create("a", json!({}));
    s.create("b", json!({}));
    assert_eq!(send(&mut s, "a", "one")["MessageId"], "m-1");
    assert_eq!(send(&mut s, "b", "two")["MessageId"], "m-2");
    assert!(send(&mut s, "a", "three").get("SequenceNumber").is_none());
    s.tick(7);
    let got = receive(&mut s, "a", 10);
    assert_eq!(
        got[0],
        json!({"MessageId": "m-1", "ReceiptHandle": "r-1", "Body": "one", "ReceiveCount": 1,
               "SentAt": 0, "FirstReceivedAt": 7})
    );
    assert_eq!(got[1]["MessageId"], "m-3");
    assert_eq!(got[1]["ReceiptHandle"], "r-2");
    // A failed send uses up no id.
    assert_eq!(
        s.err(json!({"Command": "SendMessage", "QueueName": "a", "Body": ""})),
        "InvalidParameterValue"
    );
    assert_eq!(send(&mut s, "a", "four")["MessageId"], "m-4");
}

#[test]
fn send_validation_order() {
    let mut s = Session::new();
    s.create("a", json!({}));
    s.create("f.fifo", json!({"Fifo": true}));
    let c = |v: Value| {
        let mut m = json!({"Command": "SendMessage"});
        m.as_object_mut()
            .unwrap()
            .extend(v.as_object().unwrap().clone());
        m
    };
    assert_eq!(s.err(c(json!({"Body": "x"}))), "MissingParameter");
    assert_eq!(s.err(c(json!({"QueueName": "nope"}))), "MissingParameter");
    assert_eq!(
        s.err(c(json!({"QueueName": "nope", "Body": 1}))),
        "InvalidParameterValue"
    );
    assert_eq!(
        s.err(c(json!({"QueueName": "nope", "Body": "x"}))),
        "QueueDoesNotExist"
    );
    assert_eq!(
        s.err(c(json!({"QueueName": "a", "Body": "x".repeat(1025)}))),
        "InvalidParameterValue"
    );
    s.ok(c(json!({"QueueName": "a", "Body": "é".repeat(512)})));
    assert_eq!(
        s.err(c(json!({"QueueName": "a", "Body": "é".repeat(513)}))),
        "InvalidParameterValue"
    );
    assert_eq!(
        s.err(c(
            json!({"QueueName": "a", "Body": "x", "DelaySeconds": 901})
        )),
        "InvalidParameterValue"
    );
    assert_eq!(
        s.err(c(json!({"QueueName": "a", "Body": "x", "GroupId": "a b"}))),
        "InvalidParameterValue"
    );
    assert_eq!(
        s.err(c(json!({"QueueName": "a", "Body": "x", "GroupId": ""}))),
        "InvalidParameterValue"
    );
    assert_eq!(
        s.err(c(json!({"QueueName": "a", "Body": "x", "GroupId": "g"}))),
        "UnsupportedOperation"
    );
    assert_eq!(
        s.err(c(
            json!({"QueueName": "a", "Body": "x", "DeduplicationId": "g"})
        )),
        "UnsupportedOperation"
    );
    assert_eq!(
        s.err(c(
            json!({"QueueName": "f.fifo", "Body": "x", "DelaySeconds": 0, "GroupId": "g"})
        )),
        "UnsupportedOperation"
    );
    assert_eq!(
        s.err(c(
            json!({"QueueName": "f.fifo", "Body": "x", "DeduplicationId": "d"})
        )),
        "MissingParameter"
    );
    assert_eq!(
        s.err(c(
            json!({"QueueName": "f.fifo", "Body": "x", "GroupId": "g"})
        )),
        "MissingParameter"
    );
}

#[test]
fn delay_and_visibility_timeline() {
    let mut s = Session::new();
    s.create("q", json!({"DelaySeconds": 10, "VisibilityTimeout": 20}));
    send(&mut s, "q", "queue-delay");
    s.ok(
        json!({"Command": "SendMessage", "QueueName": "q", "Body": "own-delay", "DelaySeconds": 0}),
    );
    assert_eq!(
        s.counts("q"),
        json!({"Visible": 1, "InFlight": 0, "Delayed": 1})
    );
    assert_eq!(bodies(&receive(&mut s, "q", 10)), ["own-delay"]);
    assert_eq!(
        s.counts("q"),
        json!({"Visible": 0, "InFlight": 1, "Delayed": 1})
    );
    s.tick(10);
    assert_eq!(
        s.counts("q"),
        json!({"Visible": 1, "InFlight": 1, "Delayed": 0})
    );
    s.tick(10);
    // The timed-out message keeps its place in arrival order.
    assert_eq!(
        bodies(&receive(&mut s, "q", 10)),
        ["queue-delay", "own-delay"]
    );
    let again = receive(&mut s, "q", 10);
    assert!(again.is_empty());
}

#[test]
fn receive_parameters_and_visibility_override() {
    let mut s = Session::new();
    s.create("q", json!({}));
    for i in 0..12 {
        send(&mut s, "q", &i.to_string());
    }
    assert_eq!(receive(&mut s, "q", 10).len(), 10);
    assert_eq!(
        s.err(json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 0})),
        "InvalidParameterValue"
    );
    assert_eq!(
        s.err(json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 11})),
        "InvalidParameterValue"
    );
    assert_eq!(
        s.err(json!({"Command": "ReceiveMessage", "QueueName": "q", "VisibilityTimeout": 43201})),
        "InvalidParameterValue"
    );
    // VisibilityTimeout 0: visible again at once and the handle is never usable.
    let r = s.ok(json!({"Command": "ReceiveMessage", "QueueName": "q", "VisibilityTimeout": 0}));
    let handle = r["Messages"][0]["ReceiptHandle"].clone();
    assert_eq!(r["Messages"][0]["Body"], "10");
    assert_eq!(
        s.err(json!({"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": handle})),
        "ReceiptHandleExpired"
    );
    let r = s.ok(json!({"Command": "ReceiveMessage", "QueueName": "q"}));
    assert_eq!(r["Messages"][0]["Body"], "10");
    assert_eq!(r["Messages"][0]["ReceiveCount"], 2);
    assert_eq!(r["Messages"][0]["FirstReceivedAt"], 0);
}

#[test]
fn receipt_handles() {
    let mut s = Session::new();
    s.create("q", json!({}));
    s.create("other", json!({}));
    send(&mut s, "q", "a");
    send(&mut s, "other", "b");
    let del = |s: &mut Session, q: &str, h: &str| {
        s.send(json!({"Command": "DeleteMessage", "QueueName": q, "ReceiptHandle": h}))
    };
    let code = |r: Value| r["error"]["code"].as_str().unwrap_or("ok").to_owned();

    assert_eq!(code(del(&mut s, "q", "r-1")), "ReceiptHandleInvalid");
    receive(&mut s, "q", 1);
    receive(&mut s, "other", 1);
    for bad in [
        "r-01",
        "R-1",
        "r-1 ",
        "r-+1",
        "r-",
        "1",
        "r-0",
        "r-3",
        "r--1",
        " r-1",
        "r-99999999999999999999999",
    ] {
        assert_eq!(
            code(del(&mut s, "q", bad)),
            "ReceiptHandleInvalid",
            "{bad:?}"
        );
    }
    // Issued for another queue.
    assert_eq!(code(del(&mut s, "q", "r-2")), "ReceiptHandleExpired");
    assert_eq!(code(del(&mut s, "nope", "r-1")), "QueueDoesNotExist");
    assert_eq!(
        code(s.send(json!({"Command": "DeleteMessage", "QueueName": "q"}))),
        "MissingParameter"
    );
    assert_eq!(
        code(s.send(json!({"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": 1}))),
        "InvalidParameterValue"
    );
    // Redelivery supersedes the old handle.
    s.tick(30);
    receive(&mut s, "q", 1);
    assert_eq!(code(del(&mut s, "q", "r-1")), "ReceiptHandleExpired");
    assert_eq!(code(del(&mut s, "q", "r-3")), "ok");
    assert_eq!(code(del(&mut s, "q", "r-3")), "ReceiptHandleExpired");
    assert_eq!(
        s.counts("q"),
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn change_visibility() {
    let mut s = Session::new();
    s.create("q", json!({}));
    send(&mut s, "q", "a");
    let change = |s: &mut Session, v: Value| {
        s.send(json!({"Command": "ChangeMessageVisibility", "QueueName": "q", "ReceiptHandle": "r-1", "VisibilityTimeout": v}))
    };
    assert_eq!(
        change(&mut s, json!(5))["error"]["code"],
        "ReceiptHandleInvalid"
    );
    receive(&mut s, "q", 1);
    assert_eq!(
        change(&mut s, json!(43201))["error"]["code"],
        "InvalidParameterValue"
    );
    assert_eq!(
        s.err(
            json!({"Command": "ChangeMessageVisibility", "QueueName": "q", "ReceiptHandle": "r-1"})
        ),
        "MissingParameter"
    );
    assert_eq!(change(&mut s, json!(100))["ok"], true);
    s.tick(99);
    assert_eq!(s.counts("q")["InFlight"], 1);
    s.tick(1);
    assert_eq!(s.counts("q")["Visible"], 1);
    assert_eq!(
        change(&mut s, json!(5))["error"]["code"],
        "ReceiptHandleExpired"
    );
    // 0 makes the message visible at once and keeps the receive count.
    receive(&mut s, "q", 1);
    let r = s.send(json!({"Command": "ChangeMessageVisibility", "QueueName": "q", "ReceiptHandle": "r-2", "VisibilityTimeout": 0}));
    assert_eq!(r["ok"], true);
    assert_eq!(s.counts("q")["Visible"], 1);
    assert_eq!(receive(&mut s, "q", 1)[0]["ReceiveCount"], 3);
}

#[test]
fn fifo_groups_and_sequence_numbers() {
    let mut s = Session::new();
    s.create("f.fifo", json!({"Fifo": true, "VisibilityTimeout": 10}));
    assert_eq!(
        fifo_send(&mut s, "f.fifo", "g1", "d1", "a1")["SequenceNumber"],
        1
    );
    assert_eq!(
        fifo_send(&mut s, "f.fifo", "g1", "d2", "a2")["SequenceNumber"],
        2
    );
    assert_eq!(
        fifo_send(&mut s, "f.fifo", "g2", "d3", "b1")["SequenceNumber"],
        3
    );
    assert_eq!(
        fifo_send(&mut s, "f.fifo", "g2", "d4", "b2")["SequenceNumber"],
        4
    );

    let got = receive(&mut s, "f.fifo", 10);
    assert_eq!(bodies(&got), ["a1", "b1"]);
    assert_eq!(got[0]["GroupId"], "g1");
    assert_eq!(got[1]["SequenceNumber"], 3);
    // Groups with an in-flight head deliver nothing.
    assert!(receive(&mut s, "f.fifo", 10).is_empty());
    let handle = got[0]["ReceiptHandle"].clone();
    s.ok(json!({"Command": "DeleteMessage", "QueueName": "f.fifo", "ReceiptHandle": handle}));
    assert_eq!(bodies(&receive(&mut s, "f.fifo", 10)), ["a2"]);
}

#[test]
fn fifo_group_blocked_by_earlier_message_even_when_skipped() {
    let mut s = Session::new();
    s.create("f.fifo", json!({"Fifo": true}));
    fifo_send(&mut s, "f.fifo", "g", "1", "first");
    fifo_send(&mut s, "f.fifo", "g", "2", "second");
    s.ok(json!({"Command": "ReceiveMessage", "QueueName": "f.fifo", "VisibilityTimeout": 5}));
    s.tick(5);
    // `first` is visible again and ahead of `second`.
    assert_eq!(bodies(&receive(&mut s, "f.fifo", 10)), ["first"]);
}

#[test]
fn fifo_deduplication_window() {
    let mut s = Session::new();
    s.create("f.fifo", json!({"Fifo": true}));
    s.create(
        "g.fifo",
        json!({"Fifo": true, "ContentBasedDeduplication": true}),
    );
    let first = fifo_send(&mut s, "f.fifo", "g1", "d", "one");
    assert_eq!(first["MessageId"], "m-1");
    s.tick(299);
    // Other group and body, same id: duplicate; no id is used up.
    let dup = fifo_send(&mut s, "f.fifo", "g2", "d", "two");
    assert_eq!(dup, first.clone());
    s.tick(1);
    // A duplicate does not extend the window.
    let again = fifo_send(&mut s, "f.fifo", "g1", "d", "three");
    assert_eq!(again["MessageId"], "m-2");
    assert_eq!(again["SequenceNumber"], 2);
    assert_eq!(s.counts("f.fifo")["Visible"], 2);

    // Content-based: the body is the id; an explicit id wins.
    let send = |s: &mut Session, extra: Value| {
        let mut m = json!({"Command": "SendMessage", "QueueName": "g.fifo", "GroupId": "g", "Body": "same"});
        m.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        s.ok(m)
    };
    let a = send(&mut s, json!({}));
    assert_eq!(send(&mut s, json!({})), a);
    assert_ne!(send(&mut s, json!({"DeduplicationId": "other"})), a);
}

#[test]
fn deduplication_outlives_the_message_and_purge() {
    let mut s = Session::new();
    s.create("f.fifo", json!({"Fifo": true}));
    let first = fifo_send(&mut s, "f.fifo", "g", "d", "x");
    let got = receive(&mut s, "f.fifo", 1);
    s.ok(json!({"Command": "DeleteMessage", "QueueName": "f.fifo", "ReceiptHandle": got[0]["ReceiptHandle"]}));
    assert_eq!(fifo_send(&mut s, "f.fifo", "g", "d", "x"), first);
    s.ok(json!({"Command": "PurgeQueue", "QueueName": "f.fifo"}));
    assert_eq!(fifo_send(&mut s, "f.fifo", "g", "d", "x"), first);
    // Sequence numbers carry on after a purge.
    assert_eq!(
        fifo_send(&mut s, "f.fifo", "g", "e", "y")["SequenceNumber"],
        2
    );
}

#[test]
fn purge_removes_every_state_and_invalidates_handles() {
    let mut s = Session::new();
    s.create("q", json!({}));
    s.create("other", json!({}));
    send(&mut s, "q", "a");
    s.ok(json!({"Command": "SendMessage", "QueueName": "q", "Body": "b", "DelaySeconds": 50}));
    send(&mut s, "q", "c");
    let got = receive(&mut s, "q", 1);
    s.ok(json!({"Command": "PurgeQueue", "QueueName": "q"}));
    assert_eq!(
        s.counts("q"),
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(
        s.err(json!({"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": got[0]["ReceiptHandle"]})),
        "ReceiptHandleExpired"
    );
    assert_eq!(
        s.err(json!({"Command": "PurgeQueue", "QueueName": "nope"})),
        "QueueDoesNotExist"
    );
}

#[test]
fn retention_removes_messages_in_any_state_and_stays_removed() {
    let mut s = Session::new();
    s.create("q", json!({"RetentionSeconds": 100}));
    send(&mut s, "q", "visible");
    s.ok(
        json!({"Command": "SendMessage", "QueueName": "q", "Body": "delayed", "DelaySeconds": 900}),
    );
    s.tick(50);
    send(&mut s, "q", "later");
    s.ok(json!({"Command": "ReceiveMessage", "QueueName": "q", "VisibilityTimeout": 1000}));
    s.tick(49);
    assert_eq!(
        s.counts("q"),
        json!({"Visible": 1, "InFlight": 1, "Delayed": 1})
    );
    s.tick(1);
    assert_eq!(
        s.counts("q"),
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
    // Lowering retention removes at once; raising it does not bring messages back.
    s.tick(25);
    s.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"RetentionSeconds": 60}}));
    assert_eq!(s.counts("q")["Visible"], 0);
    s.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"RetentionSeconds": 1000}}));
    assert_eq!(s.counts("q")["Visible"], 0);
}

#[test]
fn new_attributes_apply_to_later_operations_only() {
    let mut s = Session::new();
    s.create("q", json!({"VisibilityTimeout": 10}));
    send(&mut s, "q", "a");
    receive(&mut s, "q", 1);
    s.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"VisibilityTimeout": 100, "DelaySeconds": 5}}));
    s.tick(10);
    assert_eq!(
        s.counts("q"),
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
    send(&mut s, "q", "b");
    assert_eq!(s.counts("q")["Delayed"], 1);
}

#[test]
fn dead_letter_moves_after_max_receive_count() {
    let mut s = Session::new();
    s.create("dlq", json!({}));
    s.create(
        "q",
        json!({"VisibilityTimeout": 1, "DeadLetterQueue": "dlq", "MaxReceiveCount": 2}),
    );
    send(&mut s, "dlq", "already there");
    send(&mut s, "q", "poison");
    send(&mut s, "q", "fine");
    for _ in 0..2 {
        s.tick(5);
        assert_eq!(bodies(&receive(&mut s, "q", 1)), ["poison"]);
    }
    s.tick(5);
    // The move does not count toward MaxMessages: `fine` is delivered in the same call.
    assert_eq!(bodies(&receive(&mut s, "q", 1)), ["fine"]);
    assert_eq!(
        s.counts("q"),
        json!({"Visible": 0, "InFlight": 1, "Delayed": 0})
    );
    assert_eq!(s.counts("dlq")["Visible"], 2);
    let got = receive(&mut s, "dlq", 10);
    assert_eq!(bodies(&got), ["already there", "poison"]);
    assert_eq!(got[1]["MessageId"], "m-2");
    assert_eq!(got[1]["ReceiveCount"], 1);
    assert_eq!(got[1]["FirstReceivedAt"], 15);
    assert_eq!(got[1]["SentAt"], 0);
}

#[test]
fn dead_letter_fifo_group_and_sequence_and_retention() {
    let mut s = Session::new();
    s.create("d.fifo", json!({"Fifo": true, "RetentionSeconds": 60}));
    s.create("q.fifo", json!({"Fifo": true, "VisibilityTimeout": 0, "DeadLetterQueue": "d.fifo", "MaxReceiveCount": 1}));
    fifo_send(&mut s, "d.fifo", "x", "1", "existing");
    fifo_send(&mut s, "q.fifo", "g", "1", "a");
    fifo_send(&mut s, "q.fifo", "g", "2", "b");
    assert_eq!(bodies(&receive(&mut s, "q.fifo", 10)), ["a"]);
    // `a` moves out, which frees `b` in the same call.
    assert_eq!(bodies(&receive(&mut s, "q.fifo", 10)), ["b"]);
    let got = receive(&mut s, "d.fifo", 10);
    assert_eq!(bodies(&got), ["existing", "a"]);
    assert_eq!(got[1]["GroupId"], "g");
    assert_eq!(got[1]["SequenceNumber"], 2);
    assert_eq!(got[1]["ReceiveCount"], 1);
}

#[test]
fn old_message_is_dropped_when_moved_to_a_short_retention_queue() {
    let mut s = Session::new();
    s.create("dlq", json!({"RetentionSeconds": 60}));
    s.create(
        "q",
        json!({"VisibilityTimeout": 0, "DeadLetterQueue": "dlq", "MaxReceiveCount": 1}),
    );
    send(&mut s, "q", "old");
    receive(&mut s, "q", 1);
    s.tick(61);
    assert!(receive(&mut s, "q", 1).is_empty());
    assert_eq!(
        s.counts("dlq"),
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(s.counts("q")["Visible"], 0);
}

#[test]
fn dead_letter_moved_message_is_visible_at_once_and_ignores_limit() {
    let mut s = Session::new();
    s.create("dlq", json!({"DelaySeconds": 100}));
    s.create(
        "q",
        json!({"VisibilityTimeout": 0, "DeadLetterQueue": "dlq", "MaxReceiveCount": 1}),
    );
    send(&mut s, "q", "m");
    receive(&mut s, "q", 1);
    receive(&mut s, "q", 1);
    assert_eq!(
        s.counts("dlq"),
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn message_limit_is_100000_and_filling_is_fast() {
    let mut s = Session::new();
    s.create("q", json!({}));
    for _ in 0..100_000 {
        let r = s.raw("{\"Command\":\"SendMessage\",\"QueueName\":\"q\",\"Body\":\"x\"}");
        assert_eq!(r["ok"], true);
    }
    assert_eq!(
        s.err(json!({"Command": "SendMessage", "QueueName": "q", "Body": "x"})),
        "LimitExceeded"
    );
    assert_eq!(s.counts("q")["Visible"], 100_000);
    // Purging frees room again, and the next id continues the count.
    s.ok(json!({"Command": "PurgeQueue", "QueueName": "q"}));
    assert_eq!(send(&mut s, "q", "y")["MessageId"], "m-100001");
}
