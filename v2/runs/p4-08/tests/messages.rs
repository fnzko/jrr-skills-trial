mod common;

use common::{Session, err};
use serde_json::{Value, json};

fn session_with(queues: &[&str]) -> Session {
    let mut s = Session::default();
    for line in queues {
        s.ok(line);
    }
    s
}

fn standard() -> Session {
    session_with(&[r#"{"Command": "CreateQueue", "QueueName": "q"}"#])
}

fn fifo() -> Session {
    session_with(&[
        r#"{"Command": "CreateQueue", "QueueName": "f.fifo", "Attributes": {"Fifo": true, "ContentBasedDeduplication": true}}"#,
    ])
}

fn send(s: &mut Session, queue: &str, body: &str) -> Value {
    s.call(&format!(
        r#"{{"Command": "SendMessage", "QueueName": "{queue}", "Body": "{body}"}}"#
    ))
}

fn send_group(s: &mut Session, body: &str, group: &str) -> Value {
    s.call(&format!(
        r#"{{"Command": "SendMessage", "QueueName": "f.fifo", "Body": "{body}", "GroupId": "{group}"}}"#
    ))
}

fn receive(s: &mut Session, queue: &str, max: u32) -> Value {
    s.call(&format!(
        r#"{{"Command": "ReceiveMessage", "QueueName": "{queue}", "MaxMessages": {max}}}"#
    ))
}

/// The bodies of a `ReceiveMessage` response, in delivery order.
fn bodies(response: &Value) -> Vec<&str> {
    response["Messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["Body"].as_str().unwrap())
        .collect()
}

fn counts(s: &mut Session, queue: &str) -> Value {
    s.call(&format!(
        r#"{{"Command": "GetQueueAttributes", "QueueName": "{queue}"}}"#
    ))["Counts"]
        .clone()
}

fn tick(s: &mut Session, seconds: u64) {
    s.call(&format!(r#"{{"Command": "Tick", "Seconds": {seconds}}}"#));
}

fn delete(s: &mut Session, queue: &str, handle: &str) -> Value {
    s.call(&format!(
        r#"{{"Command": "DeleteMessage", "QueueName": "{queue}", "ReceiptHandle": "{handle}"}}"#
    ))
}

#[test]
fn send_then_receive_reports_every_member() {
    let mut s = standard();
    tick(&mut s, 3);
    assert_eq!(
        send(&mut s, "q", "hello"),
        json!({"ok": true, "MessageId": "m-1"})
    );
    tick(&mut s, 2);
    assert_eq!(
        receive(&mut s, "q", 10),
        json!({"ok": true, "Messages": [{
            "MessageId": "m-1",
            "ReceiptHandle": "r-1",
            "Body": "hello",
            "ReceiveCount": 1,
            "SentAt": 3,
            "FirstReceivedAt": 5,
        }]})
    );
    assert_eq!(
        receive(&mut s, "q", 10),
        json!({"ok": true, "Messages": []})
    );
}

#[test]
fn ids_count_over_all_queues_and_skip_failed_sends() {
    let mut s = session_with(&[
        r#"{"Command": "CreateQueue", "QueueName": "a"}"#,
        r#"{"Command": "CreateQueue", "QueueName": "b"}"#,
    ]);
    assert_eq!(send(&mut s, "a", "1")["MessageId"], "m-1");
    assert_eq!(
        s.call(r#"{"Command": "SendMessage", "QueueName": "a", "Body": "x", "GroupId": "g"}"#),
        err("UnsupportedOperation")
    );
    assert_eq!(send(&mut s, "b", "2")["MessageId"], "m-2");
    assert_eq!(
        receive(&mut s, "b", 1)["Messages"][0]["ReceiptHandle"],
        "r-1"
    );
    assert_eq!(
        receive(&mut s, "a", 1)["Messages"][0]["ReceiptHandle"],
        "r-2"
    );
}

#[test]
fn delayed_message_becomes_visible_when_its_delay_ends() {
    let mut s = session_with(&[
        r#"{"Command": "CreateQueue", "QueueName": "q", "Attributes": {"DelaySeconds": 10}}"#,
    ]);
    send(&mut s, "q", "queue-delay");
    s.call(
        r#"{"Command": "SendMessage", "QueueName": "q", "Body": "no-delay", "DelaySeconds": 0}"#,
    );
    assert_eq!(
        counts(&mut s, "q"),
        json!({"Visible": 1, "InFlight": 0, "Delayed": 1})
    );
    assert_eq!(bodies(&receive(&mut s, "q", 10)), ["no-delay"]);
    tick(&mut s, 9);
    assert_eq!(bodies(&receive(&mut s, "q", 10)), Vec::<&str>::new());
    tick(&mut s, 1);
    assert_eq!(bodies(&receive(&mut s, "q", 10)), ["queue-delay"]);
}

#[test]
fn in_flight_message_returns_in_arrival_order_after_its_timeout() {
    let mut s = standard();
    send(&mut s, "q", "a");
    send(&mut s, "q", "b");
    s.call(r#"{"Command": "ReceiveMessage", "QueueName": "q", "VisibilityTimeout": 5}"#);
    assert_eq!(
        counts(&mut s, "q"),
        json!({"Visible": 1, "InFlight": 1, "Delayed": 0})
    );
    tick(&mut s, 5);
    let response = receive(&mut s, "q", 10);
    assert_eq!(bodies(&response), ["a", "b"]);
    let first = &response["Messages"][0];
    assert_eq!(
        (
            first["ReceiveCount"].clone(),
            first["FirstReceivedAt"].clone(),
            first["ReceiptHandle"].clone()
        ),
        (json!(2), json!(0), json!("r-2"))
    );
}

#[test]
fn delete_message_needs_the_latest_handle_of_a_message_in_flight() {
    let mut s = standard();
    send(&mut s, "q", "a");
    receive(&mut s, "q", 1);
    assert_eq!(delete(&mut s, "q", "r-1"), json!({"ok": true}));
    assert_eq!(delete(&mut s, "q", "r-1"), err("ReceiptHandleExpired"));
    assert_eq!(
        counts(&mut s, "q"),
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );

    for never_issued in ["r-2", "r-01", "R-1", "r-1 ", ""] {
        assert_eq!(
            delete(&mut s, "q", never_issued),
            err("ReceiptHandleInvalid"),
            "handle {never_issued:?}"
        );
    }
}

#[test]
fn handle_expires_when_the_message_is_received_again_or_times_out() {
    let mut s = standard();
    send(&mut s, "q", "a");
    s.call(r#"{"Command": "ReceiveMessage", "QueueName": "q", "VisibilityTimeout": 1}"#);
    tick(&mut s, 1);
    assert_eq!(delete(&mut s, "q", "r-1"), err("ReceiptHandleExpired"));
    receive(&mut s, "q", 1);
    assert_eq!(delete(&mut s, "q", "r-1"), err("ReceiptHandleExpired"));
    assert_eq!(delete(&mut s, "q", "r-2"), json!({"ok": true}));
}

#[test]
fn handle_from_another_queue_is_expired() {
    let mut s = session_with(&[
        r#"{"Command": "CreateQueue", "QueueName": "a"}"#,
        r#"{"Command": "CreateQueue", "QueueName": "b"}"#,
    ]);
    send(&mut s, "a", "x");
    receive(&mut s, "a", 1);
    assert_eq!(delete(&mut s, "b", "r-1"), err("ReceiptHandleExpired"));
}

#[test]
fn zero_visibility_timeout_never_gives_a_usable_handle() {
    let mut s = standard();
    send(&mut s, "q", "a");
    s.call(r#"{"Command": "ReceiveMessage", "QueueName": "q", "VisibilityTimeout": 0}"#);
    assert_eq!(
        counts(&mut s, "q"),
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(delete(&mut s, "q", "r-1"), err("ReceiptHandleExpired"));
}

#[test]
fn change_message_visibility_restarts_the_timeout_from_now() {
    let mut s = standard();
    send(&mut s, "q", "a");
    receive(&mut s, "q", 1);
    tick(&mut s, 20);
    s.ok(r#"{"Command": "ChangeMessageVisibility", "QueueName": "q", "ReceiptHandle": "r-1", "VisibilityTimeout": 100}"#);
    tick(&mut s, 99);
    assert_eq!(
        counts(&mut s, "q"),
        json!({"Visible": 0, "InFlight": 1, "Delayed": 0})
    );
    s.ok(r#"{"Command": "ChangeMessageVisibility", "QueueName": "q", "ReceiptHandle": "r-1", "VisibilityTimeout": 0}"#);
    assert_eq!(
        counts(&mut s, "q"),
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(
        s.call(r#"{"Command": "ChangeMessageVisibility", "QueueName": "q", "ReceiptHandle": "r-1", "VisibilityTimeout": 5}"#),
        err("ReceiptHandleExpired")
    );
    assert_eq!(receive(&mut s, "q", 1)["Messages"][0]["ReceiveCount"], 2);
}

#[test]
fn retention_removes_messages_in_every_state() {
    let mut s = session_with(&[
        r#"{"Command": "CreateQueue", "QueueName": "q", "Attributes": {"RetentionSeconds": 100}}"#,
    ]);
    send(&mut s, "q", "in-flight");
    s.call(
        r#"{"Command": "SendMessage", "QueueName": "q", "Body": "delayed", "DelaySeconds": 900}"#,
    );
    s.call(r#"{"Command": "ReceiveMessage", "QueueName": "q", "VisibilityTimeout": 1000}"#);
    tick(&mut s, 50);
    send(&mut s, "q", "young");
    tick(&mut s, 49);
    assert_eq!(
        counts(&mut s, "q"),
        json!({"Visible": 1, "InFlight": 1, "Delayed": 1})
    );
    tick(&mut s, 1);
    assert_eq!(
        counts(&mut s, "q"),
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(delete(&mut s, "q", "r-1"), err("ReceiptHandleExpired"));
}

#[test]
fn lowering_retention_removes_old_messages_for_good() {
    let mut s = standard();
    send(&mut s, "q", "old");
    tick(&mut s, 100);
    send(&mut s, "q", "new");
    s.ok(r#"{"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"RetentionSeconds": 60}}"#);
    s.ok(r#"{"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"RetentionSeconds": 1000}}"#);
    assert_eq!(bodies(&receive(&mut s, "q", 10)), ["new"]);
}

#[test]
fn purge_removes_every_message_but_keeps_sequence_numbers() {
    let mut s = fifo();
    assert_eq!(send_group(&mut s, "a", "g")["SequenceNumber"], 1);
    receive(&mut s, "f.fifo", 1);
    s.ok(r#"{"Command": "PurgeQueue", "QueueName": "f.fifo"}"#);
    assert_eq!(
        counts(&mut s, "f.fifo"),
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(delete(&mut s, "f.fifo", "r-1"), err("ReceiptHandleExpired"));
    assert_eq!(
        send_group(&mut s, "b", "g"),
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2})
    );
}

#[test]
fn send_message_rule_violations() {
    let mut s = session_with(&[
        r#"{"Command": "CreateQueue", "QueueName": "q"}"#,
        r#"{"Command": "CreateQueue", "QueueName": "f.fifo", "Attributes": {"Fifo": true}}"#,
    ]);
    let cases = [
        (
            r#""QueueName": "q", "Body": "b", "DeduplicationId": "d""#,
            "UnsupportedOperation",
        ),
        (
            r#""QueueName": "f.fifo", "Body": "b", "DelaySeconds": 0"#,
            "UnsupportedOperation",
        ),
        (
            r#""QueueName": "f.fifo", "Body": "b", "DeduplicationId": "d""#,
            "MissingParameter",
        ),
        (
            r#""QueueName": "f.fifo", "Body": "b", "GroupId": "g""#,
            "MissingParameter",
        ),
    ];
    for (members, code) in cases {
        let line = format!(r#"{{"Command": "SendMessage", {members}}}"#);
        assert_eq!(s.call(&line), err(code), "members {members}");
    }
    assert_eq!(
        s.call(r#"{"Command": "SendMessage", "QueueName": "f.fifo", "Body": "b", "GroupId": "g", "DeduplicationId": "d"}"#),
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
}

#[test]
fn fifo_delivers_one_message_per_group_in_order() {
    let mut s = fifo();
    send_group(&mut s, "a1", "a");
    send_group(&mut s, "a2", "a");
    send_group(&mut s, "b1", "b");
    let response = receive(&mut s, "f.fifo", 10);
    assert_eq!(bodies(&response), ["a1", "b1"]);
    assert_eq!(
        (
            response["Messages"][0]["GroupId"].clone(),
            response["Messages"][0]["SequenceNumber"].clone()
        ),
        (json!("a"), json!(1))
    );
    // a2 waits while a1 is in flight, and comes once a1 is deleted.
    assert_eq!(bodies(&receive(&mut s, "f.fifo", 10)), Vec::<&str>::new());
    delete(&mut s, "f.fifo", "r-1");
    assert_eq!(bodies(&receive(&mut s, "f.fifo", 10)), ["a2"]);
}

#[test]
fn fifo_deduplicates_within_three_hundred_seconds() {
    let mut s = fifo();
    assert_eq!(
        send_group(&mut s, "same", "g"),
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
    // The id ignores the group and outlives the message.
    receive(&mut s, "f.fifo", 1);
    delete(&mut s, "f.fifo", "r-1");
    tick(&mut s, 299);
    assert_eq!(
        send_group(&mut s, "same", "other"),
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
    tick(&mut s, 1);
    assert_eq!(
        send_group(&mut s, "same", "g"),
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2})
    );
    assert_eq!(
        counts(&mut s, "f.fifo"),
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn explicit_deduplication_id_replaces_the_body() {
    let mut s = fifo();
    let line = |body: &str, id: &str| {
        format!(
            r#"{{"Command": "SendMessage", "QueueName": "f.fifo", "Body": "{body}", "GroupId": "g", "DeduplicationId": "{id}"}}"#
        )
    };
    assert_eq!(s.call(&line("x", "id1"))["MessageId"], "m-1");
    assert_eq!(s.call(&line("y", "id1"))["MessageId"], "m-1");
    assert_eq!(s.call(&line("z", "x"))["MessageId"], "m-2");
    // A body equal to an earlier explicit id is a duplicate of it.
    assert_eq!(send_group(&mut s, "id1", "g")["MessageId"], "m-1");
}

#[test]
fn message_limit_counts_every_state() {
    let mut s = standard();
    for _ in 0..100_000 {
        send(&mut s, "q", "x");
    }
    assert_eq!(send(&mut s, "q", "x"), err("LimitExceeded"));
    receive(&mut s, "q", 1);
    assert_eq!(send(&mut s, "q", "x"), err("LimitExceeded"));
    delete(&mut s, "q", "r-1");
    assert_eq!(
        send(&mut s, "q", "x"),
        json!({"ok": true, "MessageId": "m-100001"})
    );
}

fn with_dead_letter(fifo: bool, retention: u64) -> Session {
    let (q, dlq, extra) = if fifo {
        (
            "f.fifo",
            "dlq.fifo",
            r#""Fifo": true, "ContentBasedDeduplication": true, "#,
        )
    } else {
        ("q", "dlq", "")
    };
    session_with(&[
        &format!(
            r#"{{"Command": "CreateQueue", "QueueName": "{dlq}", "Attributes": {{{extra}"RetentionSeconds": {retention}}}}}"#
        ),
        &format!(
            r#"{{"Command": "CreateQueue", "QueueName": "{q}", "Attributes": {{{extra}"DeadLetterQueue": "{dlq}", "MaxReceiveCount": 2, "VisibilityTimeout": 0}}}}"#
        ),
    ])
}

#[test]
fn message_moves_to_dead_letter_queue_after_max_receives() {
    let mut s = with_dead_letter(false, 345_600);
    tick(&mut s, 7);
    send(&mut s, "q", "poison");
    send(&mut s, "q", "fine");
    assert_eq!(bodies(&receive(&mut s, "q", 1)), ["poison"]);
    assert_eq!(bodies(&receive(&mut s, "q", 1)), ["poison"]);
    // The third examination moves it and does not count toward MaxMessages.
    assert_eq!(bodies(&receive(&mut s, "q", 1)), ["fine"]);
    assert_eq!(
        counts(&mut s, "dlq"),
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
    tick(&mut s, 1);
    assert_eq!(
        receive(&mut s, "dlq", 10),
        json!({"ok": true, "Messages": [{
            "MessageId": "m-1",
            "ReceiptHandle": "r-4",
            "Body": "poison",
            "ReceiveCount": 1,
            "SentAt": 7,
            "FirstReceivedAt": 8,
        }]})
    );
}

#[test]
fn fifo_dead_letter_move_takes_a_new_sequence_number_and_unblocks_the_group() {
    let mut s = with_dead_letter(true, 345_600);
    s.call(r#"{"Command": "SendMessage", "QueueName": "dlq.fifo", "Body": "pre", "GroupId": "x"}"#);
    send_group(&mut s, "a1", "a");
    send_group(&mut s, "a2", "a");
    receive(&mut s, "f.fifo", 1);
    receive(&mut s, "f.fifo", 1);
    // a1 moves out, so a2 is the head of its group within the same receive.
    assert_eq!(bodies(&receive(&mut s, "f.fifo", 10)), ["a2"]);
    let moved = &s.call(r#"{"Command": "ReceiveMessage", "QueueName": "dlq.fifo", "MaxMessages": 10, "VisibilityTimeout": 0}"#)["Messages"];
    assert_eq!(
        (
            moved[0]["Body"].clone(),
            moved[0]["GroupId"].clone(),
            moved[0]["SequenceNumber"].clone()
        ),
        (json!("pre"), json!("x"), json!(1))
    );
    assert_eq!(
        (
            moved[1]["Body"].clone(),
            moved[1]["GroupId"].clone(),
            moved[1]["SequenceNumber"].clone()
        ),
        (json!("a1"), json!("a"), json!(2))
    );
}

#[test]
fn dead_letter_move_drops_messages_too_old_for_the_target() {
    let mut s = with_dead_letter(false, 60);
    send(&mut s, "q", "old");
    receive(&mut s, "q", 1);
    receive(&mut s, "q", 1);
    tick(&mut s, 60);
    assert_eq!(bodies(&receive(&mut s, "q", 1)), Vec::<&str>::new());
    assert_eq!(
        counts(&mut s, "q"),
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(
        counts(&mut s, "dlq"),
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn dead_letter_config_applies_from_the_next_receive() {
    let mut s = session_with(&[
        r#"{"Command": "CreateQueue", "QueueName": "dlq"}"#,
        r#"{"Command": "CreateQueue", "QueueName": "q", "Attributes": {"VisibilityTimeout": 0}}"#,
    ]);
    send(&mut s, "q", "a");
    receive(&mut s, "q", 1);
    s.ok(r#"{"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"DeadLetterQueue": "dlq", "MaxReceiveCount": 1}}"#);
    assert_eq!(bodies(&receive(&mut s, "q", 1)), Vec::<&str>::new());
    assert_eq!(bodies(&receive(&mut s, "dlq", 1)), ["a"]);
}
