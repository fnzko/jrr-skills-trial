mod common;

use common::Session;
use serde_json::json;

fn session_with(queues: &[(&str, serde_json::Value)]) -> Session {
    let mut s = Session::new();
    for (name, attributes) in queues {
        s.ok(json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attributes}));
    }
    s
}

fn send(s: &mut Session, queue: &str, body: &str) -> serde_json::Value {
    s.ok(json!({"Command": "SendMessage", "QueueName": queue, "Body": body}))
}

#[test]
fn message_ids_count_accepted_messages_across_queues() {
    let mut s = session_with(&[("a", json!({})), ("b", json!({}))]);
    assert_eq!(
        send(&mut s, "a", "x"),
        json!({"ok": true, "MessageId": "m-1"})
    );
    assert_eq!(
        s.err(json!({"Command": "SendMessage", "QueueName": "nope", "Body": "x"})),
        "QueueDoesNotExist"
    );
    assert_eq!(send(&mut s, "b", "x")["MessageId"], "m-2");
    assert_eq!(send(&mut s, "a", "x")["MessageId"], "m-3");
}

#[test]
fn body_and_parameter_validation() {
    let mut s = session_with(&[("a", json!({}))]);
    let long = "é".repeat(512);
    send(&mut s, "a", &long);
    for body in [
        json!(""),
        json!("é".repeat(513)),
        json!("a".repeat(1025)),
        json!(5),
    ] {
        assert_eq!(
            s.err(json!({"Command": "SendMessage", "QueueName": "a", "Body": body})),
            "InvalidParameterValue"
        );
    }
    assert_eq!(
        s.err(json!({"Command": "SendMessage", "QueueName": "a"})),
        "MissingParameter"
    );
    assert_eq!(
        s.err(json!({"Command": "SendMessage", "Body": "x"})),
        "MissingParameter"
    );
    for delay in [json!(-1), json!(901), json!("1")] {
        assert_eq!(s.err(json!({"Command": "SendMessage", "QueueName": "a", "Body": "x", "DelaySeconds": delay})), "InvalidParameterValue");
    }
    for bad in [json!(""), json!("a b"), json!("a".repeat(129)), json!(1)] {
        assert_eq!(
            s.err(json!({"Command": "SendMessage", "QueueName": "a", "Body": "x", "GroupId": bad})),
            "InvalidParameterValue"
        );
    }
    // Value errors are reported before the queue lookup.
    assert_eq!(
        s.err(json!({"Command": "SendMessage", "QueueName": "nope", "Body": ""})),
        "InvalidParameterValue"
    );
}

#[test]
fn standard_queue_rejects_fifo_parameters() {
    let mut s = session_with(&[("a", json!({}))]);
    for extra in [json!({"GroupId": "g"}), json!({"DeduplicationId": "d"})] {
        let mut command = json!({"Command": "SendMessage", "QueueName": "a", "Body": "x"});
        command
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        assert_eq!(s.err(command), "UnsupportedOperation");
    }
    // null counts as absent.
    s.ok(json!({"Command": "SendMessage", "QueueName": "a", "Body": "x", "GroupId": null}));
}

#[test]
fn delay_comes_from_parameter_or_queue_and_ends_on_time() {
    let mut s = session_with(&[("a", json!({"DelaySeconds": 10}))]);
    send(&mut s, "a", "queue default");
    s.ok(json!({"Command": "SendMessage", "QueueName": "a", "Body": "own", "DelaySeconds": 0}));
    s.ok(json!({"Command": "SendMessage", "QueueName": "a", "Body": "own", "DelaySeconds": 20}));
    let counts = |s: &mut Session| {
        s.ok(json!({"Command": "GetQueueAttributes", "QueueName": "a"}))["Counts"].clone()
    };
    assert_eq!(
        counts(&mut s),
        json!({"Visible": 1, "InFlight": 0, "Delayed": 2})
    );
    s.tick(9);
    assert_eq!(counts(&mut s)["Delayed"], 2);
    s.tick(1);
    assert_eq!(
        counts(&mut s),
        json!({"Visible": 2, "InFlight": 0, "Delayed": 1})
    );
    // A later change of the queue delay affects later sends only.
    s.ok(json!({"Command": "SetQueueAttributes", "QueueName": "a", "Attributes": {"DelaySeconds": 0}}));
    assert_eq!(counts(&mut s)["Delayed"], 1);
}

#[test]
fn fifo_send_rules_in_order() {
    let mut s = session_with(&[
        ("f.fifo", json!({"Fifo": true})),
        (
            "c.fifo",
            json!({"Fifo": true, "ContentBasedDeduplication": true}),
        ),
    ]);
    let cmd = |q: &str, extra: serde_json::Value| {
        let mut c = json!({"Command": "SendMessage", "QueueName": q, "Body": "x"});
        c.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        c
    };
    assert_eq!(
        s.err(cmd("f.fifo", json!({"DelaySeconds": 1}))),
        "UnsupportedOperation"
    );
    assert_eq!(
        s.err(cmd("f.fifo", json!({"DelaySeconds": 0, "GroupId": "g"}))),
        "UnsupportedOperation"
    );
    assert_eq!(
        s.err(cmd("f.fifo", json!({"DeduplicationId": "d"}))),
        "MissingParameter"
    );
    assert_eq!(
        s.err(cmd("f.fifo", json!({"GroupId": "g"}))),
        "MissingParameter"
    );
    s.ok(cmd("c.fifo", json!({"GroupId": "g"})));
    let r = s.ok(cmd(
        "f.fifo",
        json!({"GroupId": "g", "DeduplicationId": "d"}),
    ));
    assert_eq!(
        r,
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 1})
    );
}

#[test]
fn fifo_sequence_numbers_are_per_queue() {
    let mut s = session_with(&[
        (
            "a.fifo",
            json!({"Fifo": true, "ContentBasedDeduplication": true}),
        ),
        (
            "b.fifo",
            json!({"Fifo": true, "ContentBasedDeduplication": true}),
        ),
    ]);
    let send_to = |s: &mut Session, q: &str, body: &str| {
        s.ok(json!({"Command": "SendMessage", "QueueName": q, "Body": body, "GroupId": "g"}))["SequenceNumber"].clone()
    };
    assert_eq!(send_to(&mut s, "a.fifo", "1"), 1);
    assert_eq!(send_to(&mut s, "a.fifo", "2"), 2);
    assert_eq!(send_to(&mut s, "b.fifo", "1"), 1);
    s.ok(json!({"Command": "PurgeQueue", "QueueName": "a.fifo"}));
    assert_eq!(send_to(&mut s, "a.fifo", "3"), 3);
}

#[test]
fn duplicates_return_the_original_and_use_no_numbers() {
    let mut s = session_with(&[("f.fifo", json!({"Fifo": true}))]);
    let send_dedup = |s: &mut Session, group: &str, id: &str| {
        s.ok(json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "b", "GroupId": group, "DeduplicationId": id}))
    };
    let first = send_dedup(&mut s, "g1", "d");
    assert_eq!(first["MessageId"], "m-1");
    assert_eq!(send_dedup(&mut s, "g2", "d"), first);
    assert_eq!(send_dedup(&mut s, "g1", "e")["MessageId"], "m-2");
    assert_eq!(send_dedup(&mut s, "g1", "e")["SequenceNumber"], 2);
}

#[test]
fn dedup_window_is_300_seconds_and_duplicates_do_not_extend_it() {
    let mut s = session_with(&[(
        "f.fifo",
        json!({"Fifo": true, "ContentBasedDeduplication": true}),
    )]);
    let send_body = |s: &mut Session| {
        s.ok(json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "same", "GroupId": "g"}))
    };
    assert_eq!(send_body(&mut s)["MessageId"], "m-1");
    s.tick(299);
    assert_eq!(send_body(&mut s)["MessageId"], "m-1");
    s.tick(1);
    assert_eq!(send_body(&mut s)["MessageId"], "m-2");
    s.tick(299);
    assert_eq!(send_body(&mut s)["MessageId"], "m-2");
    s.tick(1);
    assert_eq!(send_body(&mut s)["MessageId"], "m-3");
}

#[test]
fn dedup_ids_outlive_messages_and_belong_to_one_queue() {
    let mut s = session_with(&[
        (
            "a.fifo",
            json!({"Fifo": true, "ContentBasedDeduplication": true}),
        ),
        (
            "b.fifo",
            json!({"Fifo": true, "ContentBasedDeduplication": true}),
        ),
    ]);
    let send_to = |s: &mut Session, q: &str| {
        s.ok(json!({"Command": "SendMessage", "QueueName": q, "Body": "same", "GroupId": "g"}))
    };
    send_to(&mut s, "a.fifo");
    s.ok(json!({"Command": "PurgeQueue", "QueueName": "a.fifo"}));
    assert_eq!(send_to(&mut s, "a.fifo")["MessageId"], "m-1");
    assert_eq!(send_to(&mut s, "b.fifo")["MessageId"], "m-2");
    // A deleted and re-created queue shares nothing.
    s.ok(json!({"Command": "DeleteQueue", "QueueName": "a.fifo"}));
    s.ok(json!({"Command": "CreateQueue", "QueueName": "a.fifo", "Attributes": {"Fifo": true, "ContentBasedDeduplication": true}}));
    assert_eq!(send_to(&mut s, "a.fifo")["SequenceNumber"], 1);
    assert_eq!(send_to(&mut s, "a.fifo")["MessageId"], "m-3");
}

#[test]
fn retention_removes_messages_and_lowering_applies_at_once() {
    let mut s = session_with(&[("a", json!({"RetentionSeconds": 100}))]);
    let held = |s: &mut Session| {
        let c = &s.ok(json!({"Command": "GetQueueAttributes", "QueueName": "a"}))["Counts"];
        c["Visible"].as_u64().unwrap()
            + c["InFlight"].as_u64().unwrap()
            + c["Delayed"].as_u64().unwrap()
    };
    send(&mut s, "a", "x");
    s.tick(99);
    assert_eq!(held(&mut s), 1);
    s.tick(1);
    assert_eq!(held(&mut s), 0);

    send(&mut s, "a", "y");
    s.tick(50);
    s.ok(json!({"Command": "SetQueueAttributes", "QueueName": "a", "Attributes": {"RetentionSeconds": 60}}));
    assert_eq!(held(&mut s), 1);
    s.tick(10);
    // Raising it later does not bring the message back, nor revive ones already past it.
    s.ok(json!({"Command": "SetQueueAttributes", "QueueName": "a", "Attributes": {"RetentionSeconds": 1000}}));
    assert_eq!(held(&mut s), 0);
}

#[test]
fn expiry_is_applied_before_a_longer_retention_takes_effect() {
    let mut s = session_with(&[("a", json!({"RetentionSeconds": 60}))]);
    send(&mut s, "a", "x");
    s.tick(60);
    // Nothing touched the queue while the message expired.
    s.ok(json!({"Command": "SetQueueAttributes", "QueueName": "a", "Attributes": {"RetentionSeconds": 1000}}));
    let r = s.ok(json!({"Command": "GetQueueAttributes", "QueueName": "a"}));
    assert_eq!(r["Counts"]["Visible"], 0);
}

#[test]
fn queue_holds_at_most_100000_messages_in_every_state() {
    let mut s = session_with(&[("a", json!({"RetentionSeconds": 60}))]);
    let line = r#"{"Command":"SendMessage","QueueName":"a","Body":"x"}"#;
    for i in 0..100_000 {
        assert_eq!(s.raw(line)["ok"], true, "send {i}");
    }
    assert_eq!(s.raw(line)["error"]["code"], "LimitExceeded");
    // A failed send used no message id.
    s.tick(60);
    assert_eq!(s.raw(line)["MessageId"], "m-100001");
}

#[test]
fn purge_empties_the_queue_in_every_state() {
    let mut s = session_with(&[("a", json!({}))]);
    send(&mut s, "a", "x");
    s.ok(json!({"Command": "SendMessage", "QueueName": "a", "Body": "y", "DelaySeconds": 5}));
    s.ok(json!({"Command": "PurgeQueue", "QueueName": "a"}));
    let r = s.ok(json!({"Command": "GetQueueAttributes", "QueueName": "a"}));
    assert_eq!(
        r["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}
