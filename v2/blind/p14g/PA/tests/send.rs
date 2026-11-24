mod common;

use common::{Session, code};
use serde_json::{Value, json};

fn counts(session: &mut Session, queue: &str) -> Value {
    let response = session.command(json!({"Command": "GetQueueAttributes", "QueueName": queue}));
    response["Counts"].clone()
}

fn send(session: &mut Session, mut command: Value) -> Value {
    command["Command"] = json!("SendMessage");
    session.command(command)
}

#[test]
fn message_ids_count_accepted_messages_over_all_queues() {
    let mut session = Session::new();
    session.create("a", json!({}));
    session.create("b", json!({}));
    let first = send(&mut session, json!({"QueueName": "a", "Body": "x"}));
    assert_eq!(first, json!({"ok": true, "MessageId": "m-1"}));
    let failed = send(&mut session, json!({"QueueName": "a", "Body": ""}));
    assert_eq!(code(&failed), "InvalidParameterValue");
    let second = send(&mut session, json!({"QueueName": "b", "Body": "y"}));
    assert_eq!(second, json!({"ok": true, "MessageId": "m-2"}));
}

#[test]
fn send_checks_required_then_values_then_queue() {
    let mut session = Session::new();
    session.create("q", json!({}));
    let cases = [
        (json!({"QueueName": "q"}), "MissingParameter"),
        (json!({"Body": "x"}), "MissingParameter"),
        (
            json!({"QueueName": "nope", "Body": ""}),
            "InvalidParameterValue",
        ),
        (
            json!({"QueueName": "nope", "Body": "x"}),
            "QueueDoesNotExist",
        ),
        (
            json!({"QueueName": "q", "Body": 5}),
            "InvalidParameterValue",
        ),
        (
            json!({"QueueName": "q", "Body": "a".repeat(1025)}),
            "InvalidParameterValue",
        ),
        (
            json!({"QueueName": "q", "Body": "x", "DelaySeconds": 901}),
            "InvalidParameterValue",
        ),
        (
            json!({"QueueName": "q", "Body": "x", "DelaySeconds": 1.5}),
            "InvalidParameterValue",
        ),
        (
            json!({"QueueName": "q", "Body": "x", "GroupId": "bad group"}),
            "InvalidParameterValue",
        ),
        (
            json!({"QueueName": "q", "Body": "x", "GroupId": "g"}),
            "UnsupportedOperation",
        ),
        (
            json!({"QueueName": "q", "Body": "x", "DeduplicationId": "d"}),
            "UnsupportedOperation",
        ),
        (
            json!({"QueueName": "q", "Body": "x", "GroupId": "a".repeat(129)}),
            "InvalidParameterValue",
        ),
    ];
    for (command, expected) in cases {
        assert_eq!(
            code(&send(&mut session, command.clone())),
            expected,
            "{command}"
        );
    }
    assert_eq!(
        counts(&mut session, "q"),
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn body_limit_is_in_bytes() {
    let mut session = Session::new();
    session.create("q", json!({}));
    let fits = send(
        &mut session,
        json!({"QueueName": "q", "Body": "\u{e9}".repeat(512)}),
    );
    assert_eq!(fits["ok"], json!(true));
    let too_big = send(
        &mut session,
        json!({"QueueName": "q", "Body": "\u{e9}".repeat(513)}),
    );
    assert_eq!(code(&too_big), "InvalidParameterValue");
}

#[test]
fn delay_comes_from_the_request_or_the_queue_at_send_time() {
    let mut session = Session::new();
    session.create("q", json!({"DelaySeconds": 10}));
    send(
        &mut session,
        json!({"QueueName": "q", "Body": "queue default"}),
    );
    send(
        &mut session,
        json!({"QueueName": "q", "Body": "none", "DelaySeconds": 0}),
    );
    send(
        &mut session,
        json!({"QueueName": "q", "Body": "long", "DelaySeconds": 100}),
    );
    assert_eq!(
        counts(&mut session, "q"),
        json!({"Visible": 1, "InFlight": 0, "Delayed": 2})
    );
    session.tick(9);
    assert_eq!(
        counts(&mut session, "q"),
        json!({"Visible": 1, "InFlight": 0, "Delayed": 2})
    );
    session.tick(1);
    assert_eq!(
        counts(&mut session, "q"),
        json!({"Visible": 2, "InFlight": 0, "Delayed": 1})
    );
    session.expect_ok(json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"DelaySeconds": 0}}));
    assert_eq!(
        counts(&mut session, "q"),
        json!({"Visible": 2, "InFlight": 0, "Delayed": 1})
    );
    session.tick(90);
    assert_eq!(
        counts(&mut session, "q"),
        json!({"Visible": 3, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn fifo_send_rules() {
    let mut session = Session::new();
    session.create("f.fifo", json!({"Fifo": true}));
    session.create(
        "c.fifo",
        json!({"Fifo": true, "ContentBasedDeduplication": true}),
    );
    let cases = [
        (
            "f.fifo",
            json!({"Body": "x", "DelaySeconds": 1, "GroupId": "g"}),
            "UnsupportedOperation",
        ),
        (
            "f.fifo",
            json!({"Body": "x", "DelaySeconds": 1}),
            "UnsupportedOperation",
        ),
        (
            "f.fifo",
            json!({"Body": "x", "DeduplicationId": "d"}),
            "MissingParameter",
        ),
        (
            "f.fifo",
            json!({"Body": "x", "GroupId": "g"}),
            "MissingParameter",
        ),
        ("c.fifo", json!({"Body": "x"}), "MissingParameter"),
    ];
    for (queue, mut command, expected) in cases {
        command["QueueName"] = json!(queue);
        assert_eq!(
            code(&send(&mut session, command.clone())),
            expected,
            "{queue} {command}"
        );
    }
    let accepted = send(
        &mut session,
        json!({"QueueName": "c.fifo", "Body": "x", "GroupId": "g"}),
    );
    assert_eq!(
        accepted,
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
}

#[test]
fn queue_delay_applies_to_fifo_sends() {
    let mut session = Session::new();
    session.create("f.fifo", json!({"Fifo": true, "DelaySeconds": 5}));
    send(
        &mut session,
        json!({"QueueName": "f.fifo", "Body": "x", "GroupId": "g", "DeduplicationId": "d"}),
    );
    assert_eq!(
        counts(&mut session, "f.fifo"),
        json!({"Visible": 0, "InFlight": 0, "Delayed": 1})
    );
}

#[test]
fn duplicate_sends_return_the_original_and_use_no_number() {
    let mut session = Session::new();
    session.create("f.fifo", json!({"Fifo": true}));
    let send_with = |session: &mut Session, id: &str, group: &str| {
        send(
            session,
            json!({"QueueName": "f.fifo", "Body": "same", "GroupId": group, "DeduplicationId": id}),
        )
    };
    assert_eq!(
        send_with(&mut session, "d1", "g1"),
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
    session.tick(299);
    assert_eq!(
        send_with(&mut session, "d1", "g2"),
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
    assert_eq!(
        send_with(&mut session, "d2", "g1"),
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2})
    );
    session.tick(1);
    // The duplicate at 299 did not extend the window that opened at 0.
    assert_eq!(
        send_with(&mut session, "d1", "g1"),
        json!({"ok": true, "MessageId": "m-3", "SequenceNumber": 3})
    );
    assert_eq!(counts(&mut session, "f.fifo")["Visible"], json!(3));
}

#[test]
fn content_based_deduplication_uses_the_body_and_explicit_ids_win() {
    let mut session = Session::new();
    session.create(
        "c.fifo",
        json!({"Fifo": true, "ContentBasedDeduplication": true}),
    );
    let long = "b".repeat(1024);
    let one = send(
        &mut session,
        json!({"QueueName": "c.fifo", "Body": long, "GroupId": "g"}),
    );
    let again = send(
        &mut session,
        json!({"QueueName": "c.fifo", "Body": long, "GroupId": "h"}),
    );
    assert_eq!(one, again);
    let explicit = send(
        &mut session,
        json!({"QueueName": "c.fifo", "Body": long, "GroupId": "g", "DeduplicationId": "own"}),
    );
    assert_eq!(
        explicit,
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2})
    );
}

#[test]
fn deduplication_ids_outlive_purged_messages_and_are_per_queue() {
    let mut session = Session::new();
    session.create("a.fifo", json!({"Fifo": true}));
    session.create("b.fifo", json!({"Fifo": true}));
    let message = |queue: &str| json!({"QueueName": queue, "Body": "x", "GroupId": "g", "DeduplicationId": "d"});
    let first = send(&mut session, message("a.fifo"));
    session.expect_ok(json!({"Command": "PurgeQueue", "QueueName": "a.fifo"}));
    assert_eq!(send(&mut session, message("a.fifo")), first);
    assert_eq!(counts(&mut session, "a.fifo")["Visible"], json!(0));
    let other = send(&mut session, message("b.fifo"));
    assert_eq!(
        other,
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 1})
    );
    session.expect_ok(json!({"Command": "PurgeQueue", "QueueName": "a.fifo"}));
    let next = send(
        &mut session,
        json!({"QueueName": "a.fifo", "Body": "x", "GroupId": "g", "DeduplicationId": "e"}),
    );
    assert_eq!(
        next,
        json!({"ok": true, "MessageId": "m-3", "SequenceNumber": 2})
    );
}

#[test]
fn deleting_a_queue_forgets_its_deduplication_ids() {
    let mut session = Session::new();
    let message =
        json!({"QueueName": "q.fifo", "Body": "x", "GroupId": "g", "DeduplicationId": "d"});
    session.create("q.fifo", json!({"Fifo": true}));
    send(&mut session, message.clone());
    session.expect_ok(json!({"Command": "DeleteQueue", "QueueName": "q.fifo"}));
    session.create("q.fifo", json!({"Fifo": true}));
    assert_eq!(
        send(&mut session, message),
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 1})
    );
}

#[test]
fn retention_removes_messages_at_the_exact_second() {
    let mut session = Session::new();
    session.create("q", json!({"RetentionSeconds": 60, "VisibilityTimeout": 0}));
    send(&mut session, json!({"QueueName": "q", "Body": "old"}));
    session.tick(30);
    send(
        &mut session,
        json!({"QueueName": "q", "Body": "new", "DelaySeconds": 5}),
    );
    session.tick(29);
    assert_eq!(
        counts(&mut session, "q"),
        json!({"Visible": 2, "InFlight": 0, "Delayed": 0})
    );
    session.tick(1);
    assert_eq!(
        counts(&mut session, "q"),
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
    session.tick(30);
    assert_eq!(
        counts(&mut session, "q"),
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn lowering_retention_removes_at_once_and_raising_it_restores_nothing() {
    let mut session = Session::new();
    session.create("q", json!({}));
    send(&mut session, json!({"QueueName": "q", "Body": "a"}));
    session.tick(100);
    send(&mut session, json!({"QueueName": "q", "Body": "b"}));
    session.expect_ok(json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"RetentionSeconds": 100}}));
    assert_eq!(counts(&mut session, "q")["Visible"], json!(1));
    session.expect_ok(json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"RetentionSeconds": 200}}));
    assert_eq!(counts(&mut session, "q")["Visible"], json!(1));
    // Expiry that happened between two commands is not undone either.
    session.tick(199);
    assert_eq!(counts(&mut session, "q")["Visible"], json!(1));
    session.tick(1);
    session.expect_ok(json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"RetentionSeconds": 1_209_600}}));
    assert_eq!(counts(&mut session, "q")["Visible"], json!(0));
}

#[test]
fn a_queue_holds_at_most_100000_messages_and_filling_it_is_fast() {
    let mut session = Session::new();
    session.create("q", json!({}));
    let line = r#"{"Command": "SendMessage", "QueueName": "q", "Body": "x"}"#;
    for _ in 0..100_000 {
        assert_eq!(session.send(line)["ok"], json!(true));
    }
    assert_eq!(code(&session.send(line)), "LimitExceeded");
    assert_eq!(counts(&mut session, "q")["Visible"], json!(100_000));
    session.expect_ok(json!({"Command": "PurgeQueue", "QueueName": "q"}));
    assert_eq!(
        session.send(line),
        json!({"ok": true, "MessageId": "m-100001"})
    );
}
