mod common;

use common::{Session, bodies};
use serde_json::{Value, json};

fn send_to(session: &mut Session, queue: &str, extra: Value) -> Value {
    let mut command = json!({"Command": "SendMessage", "QueueName": queue, "Body": "b"});
    command
        .as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    session.call(command)
}

#[test]
fn send_validates_body_and_options() {
    let mut session = Session::new();
    session.create("q", json!({}));
    let long = "x".repeat(1025);
    let cases = [
        (json!({"Body": ""}), "InvalidParameterValue"),
        (json!({"Body": long}), "InvalidParameterValue"),
        (json!({"Body": 5}), "InvalidParameterValue"),
        (json!({"DelaySeconds": 901}), "InvalidParameterValue"),
        (json!({"GroupId": ""}), "InvalidParameterValue"),
        (json!({"DeduplicationId": "a b"}), "InvalidParameterValue"),
        (json!({"GroupId": "g"}), "UnsupportedOperation"),
        (json!({"DeduplicationId": "d"}), "UnsupportedOperation"),
    ];
    for (extra, code) in cases {
        assert_eq!(
            send_to(&mut session, "q", extra.clone())["error"]["code"],
            json!(code),
            "{extra}"
        );
    }
    let bytes_not_chars = "é".repeat(512);
    assert_eq!(
        send_to(&mut session, "q", json!({"Body": bytes_not_chars}))["ok"],
        json!(true)
    );
    let too_many_bytes = "é".repeat(513);
    assert_eq!(
        send_to(&mut session, "q", json!({"Body": too_many_bytes}))["ok"],
        json!(false)
    );
}

#[test]
fn message_ids_count_only_accepted_messages() {
    let mut session = Session::new();
    session.create("q", json!({}));
    session.create("f.fifo", json!({"Fifo": true}));
    assert_eq!(session.send("q", "a"), json!({"MessageId": "m-1"}));
    assert_eq!(
        send_to(&mut session, "q", json!({"Body": ""}))["ok"],
        json!(false)
    );
    assert_eq!(session.send("q", "b"), json!({"MessageId": "m-2"}));
    let fifo = |dedup: &str| json!({"GroupId": "g", "DeduplicationId": dedup});
    assert_eq!(
        send_to(&mut session, "f.fifo", fifo("x")),
        json!({"ok": true, "MessageId": "m-3", "SequenceNumber": 1})
    );
    assert_eq!(
        send_to(&mut session, "f.fifo", fifo("x")),
        json!({"ok": true, "MessageId": "m-3", "SequenceNumber": 1})
    );
    assert_eq!(session.send("q", "c"), json!({"MessageId": "m-4"}));
}

#[test]
fn receive_returns_arrival_order_and_hides_messages_while_in_flight() {
    let mut session = Session::new();
    session.create("q", json!({"VisibilityTimeout": 10}));
    for body in ["a", "b", "c"] {
        session.send("q", body);
    }
    let first = session.receive_up_to("q", 2);
    assert_eq!(
        first,
        vec![
            json!({"MessageId": "m-1", "ReceiptHandle": "r-1", "Body": "a", "ReceiveCount": 1,
                "SentAt": 0, "FirstReceivedAt": 0}),
            json!({"MessageId": "m-2", "ReceiptHandle": "r-2", "Body": "b", "ReceiveCount": 1,
                "SentAt": 0, "FirstReceivedAt": 0}),
        ]
    );
    assert_eq!(bodies(&session.receive_up_to("q", 10)), ["c"]);
    assert_eq!(
        session.counts("q"),
        json!({"Visible": 0, "InFlight": 3, "Delayed": 0})
    );
    session.tick(10);
    // The timeouts ended: all three are back in their original places.
    assert_eq!(bodies(&session.receive_up_to("q", 10)), ["a", "b", "c"]);
}

#[test]
fn receive_validates_its_parameters() {
    let mut session = Session::new();
    session.create("q", json!({}));
    for extra in [
        json!({"MaxMessages": 0}),
        json!({"MaxMessages": 11}),
        json!({"VisibilityTimeout": 43201}),
    ] {
        let mut command = json!({"Command": "ReceiveMessage", "QueueName": "q"});
        command
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        assert_eq!(session.err(command), "InvalidParameterValue", "{extra}");
    }
}

#[test]
fn delay_applies_until_it_ends_and_the_send_can_override_the_queue() {
    let mut session = Session::new();
    session.create("q", json!({"DelaySeconds": 5}));
    session.send("q", "queue-delay");
    send_to(
        &mut session,
        "q",
        json!({"Body": "no-delay", "DelaySeconds": 0}),
    );
    assert_eq!(
        session.counts("q"),
        json!({"Visible": 1, "InFlight": 0, "Delayed": 1})
    );
    assert_eq!(bodies(&session.receive_up_to("q", 10)), ["no-delay"]);
    session.tick(4);
    assert_eq!(session.receive("q"), Vec::<Value>::new());
    session.tick(1);
    assert_eq!(bodies(&session.receive("q")), ["queue-delay"]);
}

#[test]
fn changed_queue_attributes_apply_to_later_operations_only() {
    let mut session = Session::new();
    session.create("q", json!({"VisibilityTimeout": 10, "DelaySeconds": 10}));
    session.send("q", "old");
    session.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q",
        "Attributes": {"DelaySeconds": 0, "VisibilityTimeout": 100}}));
    session.send("q", "new");
    assert_eq!(bodies(&session.receive("q")), ["new"]);
    session.tick(10);
    assert_eq!(
        session.counts("q"),
        json!({"Visible": 1, "InFlight": 1, "Delayed": 0})
    );
    session.tick(89);
    assert_eq!(
        session.counts("q"),
        json!({"Visible": 1, "InFlight": 1, "Delayed": 0})
    );
    session.tick(1);
    assert_eq!(
        session.counts("q"),
        json!({"Visible": 2, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn retention_removes_messages_in_every_state_at_the_boundary() {
    let mut session = Session::new();
    session.create(
        "q",
        json!({"RetentionSeconds": 60, "DelaySeconds": 900, "VisibilityTimeout": 100}),
    );
    session.send("q", "delayed");
    send_to(&mut session, "q", json!({"DelaySeconds": 0}));
    session.receive("q");
    session.send("q", "delayed too");
    session.tick(59);
    assert_eq!(
        session.counts("q"),
        json!({"Visible": 0, "InFlight": 1, "Delayed": 2})
    );
    session.tick(1);
    assert_eq!(
        session.counts("q"),
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn lowering_retention_removes_at_once_and_raising_it_does_not_restore() {
    let mut session = Session::new();
    session.create("q", json!({}));
    session.send("q", "a");
    session.tick(100);
    session.send("q", "b");
    let set = |seconds: u32| {
        json!({"Command": "SetQueueAttributes", "QueueName": "q",
        "Attributes": {"RetentionSeconds": seconds}})
    };
    session.ok(set(100));
    assert_eq!(
        session.counts("q"),
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
    session.ok(set(1000));
    assert_eq!(bodies(&session.receive("q")), ["b"]);
}

#[test]
fn zero_visibility_timeout_makes_the_handle_unusable() {
    let mut session = Session::new();
    session.create("q", json!({}));
    session.send("q", "a");
    let messages = session.receive_up_to("q", 1);
    session.ok(
        json!({"Command": "ChangeMessageVisibility", "QueueName": "q",
        "ReceiptHandle": messages[0]["ReceiptHandle"], "VisibilityTimeout": 0}),
    );
    assert_eq!(
        session.err(json!({"Command": "DeleteMessage", "QueueName": "q",
            "ReceiptHandle": messages[0]["ReceiptHandle"]})),
        "ReceiptHandleExpired"
    );
    let command = json!({"Command": "ReceiveMessage", "QueueName": "q", "VisibilityTimeout": 0});
    let again = session.ok(command.clone());
    assert_eq!(again["Messages"][0]["ReceiveCount"], json!(2));
    assert_eq!(again["Messages"][0]["FirstReceivedAt"], json!(0));
    assert_eq!(
        session.err(json!({"Command": "DeleteMessage", "QueueName": "q",
            "ReceiptHandle": again["Messages"][0]["ReceiptHandle"]})),
        "ReceiptHandleExpired"
    );
    assert_eq!(session.ok(command)["Messages"][0]["ReceiveCount"], json!(3));
}

#[test]
fn receipt_handle_errors() {
    let mut session = Session::new();
    session.create("q", json!({"VisibilityTimeout": 5}));
    session.create("other", json!({}));
    session.send("q", "a");
    let handle = session.receive("q")[0]["ReceiptHandle"].clone();
    let delete = |queue: &str, handle: Value| json!({"Command": "DeleteMessage", "QueueName": queue, "ReceiptHandle": handle});
    for never_issued in [
        "r-01",
        "R-1",
        "r-1 ",
        "r-2",
        "r-0",
        "x",
        "",
        "r-99999999999999999999",
    ] {
        assert_eq!(
            session.err(delete("q", json!(never_issued))),
            "ReceiptHandleInvalid",
            "{never_issued:?}"
        );
    }
    assert_eq!(
        session.err(delete("other", handle.clone())),
        "ReceiptHandleExpired"
    );
    session.tick(5);
    assert_eq!(
        session.err(delete("q", handle.clone())),
        "ReceiptHandleExpired"
    );
    let newest = session.receive("q")[0]["ReceiptHandle"].clone();
    assert_eq!(newest, json!("r-2"));
    assert_eq!(
        session.err(delete("q", handle.clone())),
        "ReceiptHandleExpired"
    );
    assert_eq!(session.err(delete("missing", handle)), "QueueDoesNotExist");
    session.ok(delete("q", newest.clone()));
    assert_eq!(session.err(delete("q", newest)), "ReceiptHandleExpired");
    assert_eq!(
        session.counts("q"),
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn change_visibility_replaces_the_end_of_the_timeout() {
    let mut session = Session::new();
    session.create("q", json!({"VisibilityTimeout": 10}));
    session.send("q", "a");
    let handle = session.receive("q")[0]["ReceiptHandle"].clone();
    session.tick(8);
    let change = |seconds: u32| {
        json!({"Command": "ChangeMessageVisibility", "QueueName": "q",
        "ReceiptHandle": handle, "VisibilityTimeout": seconds})
    };
    session.ok(change(5));
    session.tick(4);
    assert_eq!(session.counts("q")["InFlight"], json!(1));
    session.ok(change(1));
    session.tick(1);
    assert_eq!(session.counts("q")["Visible"], json!(1));
    assert_eq!(session.err(change(1)), "ReceiptHandleExpired");
    assert_eq!(session.err(change(43201)), "InvalidParameterValue");
    assert_eq!(
        session.err(
            json!({"Command": "ChangeMessageVisibility", "QueueName": "q", "ReceiptHandle": handle})
        ),
        "MissingParameter"
    );
}

#[test]
fn fifo_requires_group_and_deduplication_and_rejects_delay() {
    let mut session = Session::new();
    session.create("f.fifo", json!({"Fifo": true}));
    session.create(
        "c.fifo",
        json!({"Fifo": true, "ContentBasedDeduplication": true}),
    );
    let code = |session: &mut Session, queue: &str, extra: Value| {
        send_to(session, queue, extra)["error"]["code"].clone()
    };
    assert_eq!(
        code(&mut session, "f.fifo", json!({})),
        json!("MissingParameter")
    );
    assert_eq!(
        code(&mut session, "f.fifo", json!({"GroupId": "g"})),
        json!("MissingParameter")
    );
    assert_eq!(
        code(&mut session, "f.fifo", json!({"DeduplicationId": "d"})),
        json!("MissingParameter")
    );
    assert_eq!(
        code(
            &mut session,
            "f.fifo",
            json!({"GroupId": "g", "DeduplicationId": "d", "DelaySeconds": 0})
        ),
        json!("UnsupportedOperation")
    );
    assert_eq!(
        send_to(&mut session, "c.fifo", json!({"GroupId": "g"})),
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
}

#[test]
fn fifo_delivers_one_message_per_group_and_blocks_groups_with_unavailable_heads() {
    let mut session = Session::new();
    session.create("f.fifo", json!({"Fifo": true, "VisibilityTimeout": 10}));
    let send = |session: &mut Session, id: &str, group: &str| {
        send_to(
            session,
            "f.fifo",
            json!({"Body": id, "GroupId": group, "DeduplicationId": id}),
        );
    };
    for (id, group) in [
        ("a1", "a"),
        ("b1", "b"),
        ("a2", "a"),
        ("b2", "b"),
        ("c1", "c"),
    ] {
        send(&mut session, id, group);
    }
    let first = session.receive_up_to("f.fifo", 10);
    assert_eq!(bodies(&first), ["a1", "b1", "c1"]);
    assert_eq!(first[1]["GroupId"], json!("b"));
    assert_eq!(first[1]["SequenceNumber"], json!(2));
    // Heads are in flight, so their groups stay blocked.
    assert_eq!(session.receive_up_to("f.fifo", 10), Vec::<Value>::new());
    session.ok(json!({"Command": "DeleteMessage", "QueueName": "f.fifo",
        "ReceiptHandle": first[0]["ReceiptHandle"]}));
    assert_eq!(bodies(&session.receive_up_to("f.fifo", 10)), ["a2"]);
}

#[test]
fn deduplication_window_is_300_seconds_and_is_not_extended_by_duplicates() {
    let mut session = Session::new();
    session.create("f.fifo", json!({"Fifo": true}));
    let send = |session: &mut Session, group: &str| {
        send_to(
            session,
            "f.fifo",
            json!({"GroupId": group, "DeduplicationId": "d"}),
        )
    };
    assert_eq!(send(&mut session, "g1")["MessageId"], json!("m-1"));
    session.tick(299);
    assert_eq!(send(&mut session, "g2")["MessageId"], json!("m-1"));
    session.tick(1);
    assert_eq!(send(&mut session, "g2")["MessageId"], json!("m-2"));
    session.tick(299);
    assert_eq!(send(&mut session, "g2")["MessageId"], json!("m-2"));
    // The original is gone, but its id is not free.
    session.receive("f.fifo");
    session.ok(json!({"Command": "PurgeQueue", "QueueName": "f.fifo"}));
    assert_eq!(send(&mut session, "g2")["SequenceNumber"], json!(2));
}

#[test]
fn content_based_deduplication_uses_the_body_and_ignores_the_group() {
    let mut session = Session::new();
    session.create(
        "f.fifo",
        json!({"Fifo": true, "ContentBasedDeduplication": true}),
    );
    let body = "same body";
    let first = send_to(
        &mut session,
        "f.fifo",
        json!({"Body": body, "GroupId": "g1"}),
    );
    let second = send_to(
        &mut session,
        "f.fifo",
        json!({"Body": body, "GroupId": "g2"}),
    );
    let third = send_to(
        &mut session,
        "f.fifo",
        json!({"Body": "other", "GroupId": "g1"}),
    );
    assert_eq!(first, second);
    assert_eq!(third["SequenceNumber"], json!(2));
    // An explicit id is looked up on its own, not as body text.
    let explicit = send_to(
        &mut session,
        "f.fifo",
        json!({"Body": "x", "GroupId": "g", "DeduplicationId": "same-id"}),
    );
    assert_eq!(explicit["MessageId"], json!("m-3"));
}

#[test]
fn messages_move_to_the_dead_letter_queue_after_max_receive_count() {
    let mut session = Session::new();
    session.create("dlq", json!({"RetentionSeconds": 1000}));
    session.create(
        "q",
        json!({"VisibilityTimeout": 0, "DeadLetterQueue": "dlq", "MaxReceiveCount": 2}),
    );
    session.send("q", "poison");
    session.send("q", "fine");
    assert_eq!(bodies(&session.receive("q")), ["poison"]);
    assert_eq!(bodies(&session.receive("q")), ["poison"]);
    // The third scan moves "poison" away without counting it, then delivers "fine".
    assert_eq!(bodies(&session.receive("q")), ["fine"]);
    assert_eq!(
        session.counts("q"),
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
    let moved = session.receive("dlq");
    assert_eq!(
        moved,
        vec![
            json!({"MessageId": "m-1", "ReceiptHandle": "r-4", "Body": "poison", "ReceiveCount": 1,
            "SentAt": 0, "FirstReceivedAt": 0})
        ]
    );
}

#[test]
fn dead_letter_arrival_keeps_age_and_can_expire_at_once() {
    let mut session = Session::new();
    session.create("dlq", json!({"RetentionSeconds": 100}));
    session.create(
        "q",
        json!({"VisibilityTimeout": 0, "DeadLetterQueue": "dlq", "MaxReceiveCount": 1}),
    );
    session.send("q", "old");
    session.receive("q");
    session.tick(100);
    assert_eq!(session.receive("q"), Vec::<Value>::new());
    assert_eq!(
        session.counts("dlq"),
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(
        session.counts("q"),
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn fifo_dead_letter_move_renumbers_and_unblocks_the_group() {
    let mut session = Session::new();
    session.create("d.fifo", json!({"Fifo": true}));
    session.create(
        "q.fifo",
        json!({"Fifo": true, "VisibilityTimeout": 0,
        "DeadLetterQueue": "d.fifo", "MaxReceiveCount": 1}),
    );
    for body in ["first", "second"] {
        send_to(
            &mut session,
            "q.fifo",
            json!({"Body": body, "GroupId": "g", "DeduplicationId": body}),
        );
    }
    assert_eq!(bodies(&session.receive("q.fifo")), ["first"]);
    // "first" is moved, so "second" is no longer behind it and is delivered.
    assert_eq!(bodies(&session.receive("q.fifo")), ["second"]);
    let moved = session.receive("d.fifo");
    assert_eq!(moved[0]["Body"], json!("first"));
    assert_eq!(moved[0]["SequenceNumber"], json!(1));
    assert_eq!(moved[0]["GroupId"], json!("g"));
    // Moves do not record deduplication ids in the dead-letter queue.
    let resend = send_to(
        &mut session,
        "d.fifo",
        json!({"Body": "first", "GroupId": "g", "DeduplicationId": "first"}),
    );
    assert_eq!(resend["SequenceNumber"], json!(2));
}

#[test]
fn a_full_queue_rejects_sends_but_dead_letters_still_arrive() {
    let mut session = Session::new();
    session.create("q", json!({}));
    for _ in 0..100_000 {
        let response =
            session.call(json!({"Command": "SendMessage", "QueueName": "q", "Body": "x"}));
        assert_eq!(response["ok"], json!(true));
    }
    assert_eq!(
        session.err(json!({"Command": "SendMessage", "QueueName": "q", "Body": "x"})),
        "LimitExceeded"
    );
    assert_eq!(session.counts("q")["Visible"], json!(100_000));
    session.create("dlq", json!({}));
    session.ok(json!({"Command": "SetQueueAttributes", "QueueName": "dlq",
        "Attributes": {"DeadLetterQueue": "q", "MaxReceiveCount": 1}}));
    session.send("dlq", "m");
    session.receive("dlq");
    session.ok(
        json!({"Command": "ChangeMessageVisibility", "QueueName": "dlq",
        "ReceiptHandle": "r-1", "VisibilityTimeout": 0}),
    );
    assert_eq!(session.receive("dlq"), Vec::<Value>::new());
    assert_eq!(session.counts("q")["Visible"], json!(100_001));
}
