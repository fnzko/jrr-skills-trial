//! SendMessage (section 6), retention (section 4) and the delay that a send
//! starts with.

mod common;

use common::{fail, ok};
use miniq::Broker;
use serde_json::json;

fn standard(broker: &mut Broker, name: &str) {
    ok(
        broker,
        &format!(r#"{{"Command": "CreateQueue", "QueueName": "{name}"}}"#),
    );
}

fn fifo(broker: &mut Broker, name: &str) {
    ok(
        broker,
        &format!(
            r#"{{"Command": "CreateQueue", "QueueName": "{name}", "Attributes": {{"Fifo": true}}}}"#
        ),
    );
}

fn counts(broker: &mut Broker, name: &str) -> serde_json::Value {
    ok(
        broker,
        &format!(r#"{{"Command": "GetQueueAttributes", "QueueName": "{name}"}}"#),
    )["Counts"]
        .clone()
}

#[test]
fn sends_get_increasing_message_ids_across_queues() {
    let mut broker = Broker::new();
    standard(&mut broker, "a");
    standard(&mut broker, "b");
    assert_eq!(
        ok(
            &mut broker,
            r#"{"Command": "SendMessage", "QueueName": "a", "Body": "x"}"#
        )["MessageId"],
        "m-1"
    );
    assert_eq!(
        ok(
            &mut broker,
            r#"{"Command": "SendMessage", "QueueName": "b", "Body": "y"}"#
        )["MessageId"],
        "m-2"
    );
    let response = ok(
        &mut broker,
        r#"{"Command": "SendMessage", "QueueName": "a", "Body": "z"}"#,
    );
    assert_eq!(response, json!({"ok": true, "MessageId": "m-3"}));
}

#[test]
fn a_standard_send_has_no_sequence_number() {
    let mut broker = Broker::new();
    standard(&mut broker, "a");
    let response = ok(
        &mut broker,
        r#"{"Command": "SendMessage", "QueueName": "a", "Body": "x"}"#,
    );
    assert!(response.get("SequenceNumber").is_none());
}

#[test]
fn failed_sends_do_not_use_up_a_message_id() {
    let mut broker = Broker::new();
    standard(&mut broker, "a");
    fail(
        &mut broker,
        r#"{"Command": "SendMessage", "QueueName": "missing", "Body": "x"}"#,
        "QueueDoesNotExist",
    );
    fail(
        &mut broker,
        r#"{"Command": "SendMessage", "QueueName": "a", "Body": ""}"#,
        "InvalidParameterValue",
    );
    assert_eq!(
        ok(
            &mut broker,
            r#"{"Command": "SendMessage", "QueueName": "a", "Body": "x"}"#
        )["MessageId"],
        "m-1"
    );
}

#[test]
fn body_is_limited_to_1024_utf8_bytes() {
    let mut broker = Broker::new();
    standard(&mut broker, "a");
    let at_limit = "é".repeat(512);
    ok(
        &mut broker,
        &format!(r#"{{"Command": "SendMessage", "QueueName": "a", "Body": "{at_limit}"}}"#),
    );
    let over = "é".repeat(513);
    fail(
        &mut broker,
        &format!(r#"{{"Command": "SendMessage", "QueueName": "a", "Body": "{over}"}}"#),
        "InvalidParameterValue",
    );
}

#[test]
fn required_parameters_and_their_types_are_checked_in_order() {
    let mut broker = Broker::new();
    standard(&mut broker, "a");
    fail(
        &mut broker,
        r#"{"Command": "SendMessage", "QueueName": 1}"#,
        "MissingParameter",
    );
    fail(
        &mut broker,
        r#"{"Command": "SendMessage", "QueueName": 1, "Body": "x"}"#,
        "InvalidParameterValue",
    );
    fail(
        &mut broker,
        r#"{"Command": "SendMessage", "QueueName": "a", "Body": "x", "DelaySeconds": 901}"#,
        "InvalidParameterValue",
    );
}

#[test]
fn group_and_deduplication_ids_are_syntax_checked() {
    let mut broker = Broker::new();
    fifo(&mut broker, "a.fifo");
    fail(
        &mut broker,
        r#"{"Command": "SendMessage", "QueueName": "a.fifo", "Body": "x", "GroupId": "a b"}"#,
        "InvalidParameterValue",
    );
    let long = "g".repeat(129);
    fail(
        &mut broker,
        &format!(
            r#"{{"Command": "SendMessage", "QueueName": "a.fifo", "Body": "x", "GroupId": "{long}"}}"#
        ),
        "InvalidParameterValue",
    );
}

#[test]
fn group_and_deduplication_ids_need_a_fifo_queue() {
    let mut broker = Broker::new();
    standard(&mut broker, "a");
    fail(
        &mut broker,
        r#"{"Command": "SendMessage", "QueueName": "a", "Body": "x", "GroupId": "g"}"#,
        "UnsupportedOperation",
    );
    fail(
        &mut broker,
        r#"{"Command": "SendMessage", "QueueName": "a", "Body": "x", "DeduplicationId": "d"}"#,
        "UnsupportedOperation",
    );
}

#[test]
fn fifo_sends_need_a_group_and_a_deduplication_id_unless_content_based() {
    let mut broker = Broker::new();
    fifo(&mut broker, "plain.fifo");
    fail(
        &mut broker,
        r#"{"Command": "SendMessage", "QueueName": "plain.fifo", "Body": "x", "DeduplicationId": "d"}"#,
        "MissingParameter",
    );
    fail(
        &mut broker,
        r#"{"Command": "SendMessage", "QueueName": "plain.fifo", "Body": "x", "GroupId": "g"}"#,
        "MissingParameter",
    );

    ok(
        &mut broker,
        r#"{"Command": "CreateQueue", "QueueName": "cbd.fifo", "Attributes": {"Fifo": true, "ContentBasedDeduplication": true}}"#,
    );
    ok(
        &mut broker,
        r#"{"Command": "SendMessage", "QueueName": "cbd.fifo", "Body": "x", "GroupId": "g"}"#,
    );
}

#[test]
fn fifo_sends_return_sequence_numbers_and_refuse_delays() {
    let mut broker = Broker::new();
    fifo(&mut broker, "a.fifo");
    let first = ok(
        &mut broker,
        r#"{"Command": "SendMessage", "QueueName": "a.fifo", "Body": "x", "GroupId": "g", "DeduplicationId": "1"}"#,
    );
    let second = ok(
        &mut broker,
        r#"{"Command": "SendMessage", "QueueName": "a.fifo", "Body": "y", "GroupId": "g", "DeduplicationId": "2"}"#,
    );
    assert_eq!(first["SequenceNumber"], 1);
    assert_eq!(second["SequenceNumber"], 2);
    fail(
        &mut broker,
        r#"{"Command": "SendMessage", "QueueName": "a.fifo", "Body": "x", "GroupId": "g", "DeduplicationId": "3", "DelaySeconds": 1}"#,
        "UnsupportedOperation",
    );
}

#[test]
fn a_duplicate_within_five_minutes_returns_the_original() {
    let mut broker = Broker::new();
    fifo(&mut broker, "a.fifo");
    let original = ok(
        &mut broker,
        r#"{"Command": "SendMessage", "QueueName": "a.fifo", "Body": "x", "GroupId": "g", "DeduplicationId": "d"}"#,
    );
    ok(&mut broker, r#"{"Command": "Tick", "Seconds": 299}"#);
    let duplicate = ok(
        &mut broker,
        r#"{"Command": "SendMessage", "QueueName": "a.fifo", "Body": "other", "GroupId": "g", "DeduplicationId": "d"}"#,
    );
    assert_eq!(duplicate, original);
    assert_eq!(counts(&mut broker, "a.fifo")["Visible"], 1);
}

#[test]
fn the_window_is_not_extended_by_duplicates() {
    let mut broker = Broker::new();
    fifo(&mut broker, "a.fifo");
    ok(
        &mut broker,
        r#"{"Command": "SendMessage", "QueueName": "a.fifo", "Body": "x", "GroupId": "g", "DeduplicationId": "d"}"#,
    );
    ok(&mut broker, r#"{"Command": "Tick", "Seconds": 200}"#);
    ok(
        &mut broker,
        r#"{"Command": "SendMessage", "QueueName": "a.fifo", "Body": "x", "GroupId": "g", "DeduplicationId": "d"}"#,
    );
    ok(&mut broker, r#"{"Command": "Tick", "Seconds": 100}"#);
    let fresh = ok(
        &mut broker,
        r#"{"Command": "SendMessage", "QueueName": "a.fifo", "Body": "x", "GroupId": "g", "DeduplicationId": "d"}"#,
    );
    assert_eq!(
        fresh["MessageId"], "m-2",
        "accepted at t+300, so a new message"
    );
    assert_eq!(fresh["SequenceNumber"], 2);
}

#[test]
fn a_send_after_the_window_is_accepted() {
    let mut broker = Broker::new();
    fifo(&mut broker, "a.fifo");
    ok(
        &mut broker,
        r#"{"Command": "SendMessage", "QueueName": "a.fifo", "Body": "x", "GroupId": "g", "DeduplicationId": "d"}"#,
    );
    ok(&mut broker, r#"{"Command": "Tick", "Seconds": 300}"#);
    let response = ok(
        &mut broker,
        r#"{"Command": "SendMessage", "QueueName": "a.fifo", "Body": "x", "GroupId": "g", "DeduplicationId": "d"}"#,
    );
    assert_eq!(response["MessageId"], "m-2");
}

#[test]
fn deduplication_ids_are_per_queue_and_ignore_the_group() {
    let mut broker = Broker::new();
    fifo(&mut broker, "a.fifo");
    fifo(&mut broker, "b.fifo");
    let first = ok(
        &mut broker,
        r#"{"Command": "SendMessage", "QueueName": "a.fifo", "Body": "x", "GroupId": "g1", "DeduplicationId": "d"}"#,
    );
    let other_queue = ok(
        &mut broker,
        r#"{"Command": "SendMessage", "QueueName": "b.fifo", "Body": "x", "GroupId": "g1", "DeduplicationId": "d"}"#,
    );
    let other_group = ok(
        &mut broker,
        r#"{"Command": "SendMessage", "QueueName": "a.fifo", "Body": "x", "GroupId": "g2", "DeduplicationId": "d"}"#,
    );
    assert_eq!(first["MessageId"], "m-1");
    assert_eq!(other_queue["MessageId"], "m-2");
    assert_eq!(
        other_group, first,
        "same deduplication id in the same queue is a duplicate"
    );
}

#[test]
fn content_based_deduplication_uses_the_body() {
    let mut broker = Broker::new();
    ok(
        &mut broker,
        r#"{"Command": "CreateQueue", "QueueName": "c.fifo", "Attributes": {"Fifo": true, "ContentBasedDeduplication": true}}"#,
    );
    let first = ok(
        &mut broker,
        r#"{"Command": "SendMessage", "QueueName": "c.fifo", "Body": "same", "GroupId": "g"}"#,
    );
    let again = ok(
        &mut broker,
        r#"{"Command": "SendMessage", "QueueName": "c.fifo", "Body": "same", "GroupId": "g"}"#,
    );
    let different = ok(
        &mut broker,
        r#"{"Command": "SendMessage", "QueueName": "c.fifo", "Body": "other", "GroupId": "g"}"#,
    );
    assert_eq!(again, first);
    assert_eq!(different["MessageId"], "m-2");
}

#[test]
fn an_explicit_deduplication_id_overrides_the_body() {
    let mut broker = Broker::new();
    ok(
        &mut broker,
        r#"{"Command": "CreateQueue", "QueueName": "c.fifo", "Attributes": {"Fifo": true, "ContentBasedDeduplication": true}}"#,
    );
    let first = ok(
        &mut broker,
        r#"{"Command": "SendMessage", "QueueName": "c.fifo", "Body": "one", "GroupId": "g", "DeduplicationId": "k"}"#,
    );
    let second = ok(
        &mut broker,
        r#"{"Command": "SendMessage", "QueueName": "c.fifo", "Body": "two", "GroupId": "g", "DeduplicationId": "k"}"#,
    );
    assert_eq!(second, first);
}

#[test]
fn deduplication_outlives_purge_but_not_the_queue() {
    let mut broker = Broker::new();
    fifo(&mut broker, "a.fifo");
    let first = ok(
        &mut broker,
        r#"{"Command": "SendMessage", "QueueName": "a.fifo", "Body": "x", "GroupId": "g", "DeduplicationId": "d"}"#,
    );
    ok(
        &mut broker,
        r#"{"Command": "PurgeQueue", "QueueName": "a.fifo"}"#,
    );
    let after_purge = ok(
        &mut broker,
        r#"{"Command": "SendMessage", "QueueName": "a.fifo", "Body": "x", "GroupId": "g", "DeduplicationId": "d"}"#,
    );
    assert_eq!(after_purge, first);

    ok(
        &mut broker,
        r#"{"Command": "DeleteQueue", "QueueName": "a.fifo"}"#,
    );
    fifo(&mut broker, "a.fifo");
    let fresh = ok(
        &mut broker,
        r#"{"Command": "SendMessage", "QueueName": "a.fifo", "Body": "x", "GroupId": "g", "DeduplicationId": "d"}"#,
    );
    assert_eq!(
        fresh["MessageId"], "m-2",
        "the duplicate used no message id"
    );
    assert_eq!(fresh["SequenceNumber"], 1);
}

#[test]
fn delayed_messages_count_as_delayed_until_their_time() {
    let mut broker = Broker::new();
    standard(&mut broker, "a");
    ok(
        &mut broker,
        r#"{"Command": "SendMessage", "QueueName": "a", "Body": "x", "DelaySeconds": 10}"#,
    );
    assert_eq!(
        counts(&mut broker, "a"),
        json!({"Visible": 0, "InFlight": 0, "Delayed": 1})
    );
    ok(&mut broker, r#"{"Command": "Tick", "Seconds": 10}"#);
    assert_eq!(
        counts(&mut broker, "a"),
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn a_send_without_delay_uses_the_queue_delay_at_that_moment() {
    let mut broker = Broker::new();
    ok(
        &mut broker,
        r#"{"Command": "CreateQueue", "QueueName": "a", "Attributes": {"DelaySeconds": 5}}"#,
    );
    ok(
        &mut broker,
        r#"{"Command": "SendMessage", "QueueName": "a", "Body": "x"}"#,
    );
    ok(
        &mut broker,
        r#"{"Command": "SetQueueAttributes", "QueueName": "a", "Attributes": {"DelaySeconds": 0}}"#,
    );
    ok(
        &mut broker,
        r#"{"Command": "SendMessage", "QueueName": "a", "Body": "y"}"#,
    );
    assert_eq!(
        counts(&mut broker, "a"),
        json!({"Visible": 1, "InFlight": 0, "Delayed": 1})
    );
}

#[test]
fn retention_removes_messages_at_sent_at_plus_retention() {
    let mut broker = Broker::new();
    ok(
        &mut broker,
        r#"{"Command": "CreateQueue", "QueueName": "a", "Attributes": {"RetentionSeconds": 60}}"#,
    );
    ok(
        &mut broker,
        r#"{"Command": "SendMessage", "QueueName": "a", "Body": "x"}"#,
    );
    ok(&mut broker, r#"{"Command": "Tick", "Seconds": 59}"#);
    assert_eq!(counts(&mut broker, "a")["Visible"], 1);
    ok(&mut broker, r#"{"Command": "Tick", "Seconds": 1}"#);
    assert_eq!(counts(&mut broker, "a")["Visible"], 0);
}

#[test]
fn lowering_retention_removes_old_messages_at_once_and_raising_does_not_restore_them() {
    let mut broker = Broker::new();
    standard(&mut broker, "a");
    ok(
        &mut broker,
        r#"{"Command": "SendMessage", "QueueName": "a", "Body": "x"}"#,
    );
    ok(&mut broker, r#"{"Command": "Tick", "Seconds": 100}"#);
    ok(
        &mut broker,
        r#"{"Command": "SetQueueAttributes", "QueueName": "a", "Attributes": {"RetentionSeconds": 60}}"#,
    );
    assert_eq!(counts(&mut broker, "a")["Visible"], 0);
    ok(
        &mut broker,
        r#"{"Command": "SetQueueAttributes", "QueueName": "a", "Attributes": {"RetentionSeconds": 345600}}"#,
    );
    assert_eq!(counts(&mut broker, "a")["Visible"], 0);
}

#[test]
fn an_expired_message_still_answers_a_duplicate_inside_the_window() {
    let mut broker = Broker::new();
    ok(
        &mut broker,
        r#"{"Command": "CreateQueue", "QueueName": "a.fifo", "Attributes": {"Fifo": true, "RetentionSeconds": 60}}"#,
    );
    let original = ok(
        &mut broker,
        r#"{"Command": "SendMessage", "QueueName": "a.fifo", "Body": "x", "GroupId": "g", "DeduplicationId": "d"}"#,
    );
    ok(&mut broker, r#"{"Command": "Tick", "Seconds": 60}"#);
    assert_eq!(counts(&mut broker, "a.fifo")["Visible"], 0);
    let response = ok(
        &mut broker,
        r#"{"Command": "SendMessage", "QueueName": "a.fifo", "Body": "x", "GroupId": "g", "DeduplicationId": "d"}"#,
    );
    assert_eq!(
        response, original,
        "the window is about accepted sends, not about the message still existing"
    );
    assert_eq!(counts(&mut broker, "a.fifo")["Visible"], 0);
}

#[test]
fn expired_messages_do_not_count_toward_the_limit() {
    let mut broker = Broker::new();
    ok(
        &mut broker,
        r#"{"Command": "CreateQueue", "QueueName": "a", "Attributes": {"RetentionSeconds": 60}}"#,
    );
    for _ in 0..100_000 {
        ok(
            &mut broker,
            r#"{"Command": "SendMessage", "QueueName": "a", "Body": "x"}"#,
        );
    }
    ok(&mut broker, r#"{"Command": "Tick", "Seconds": 60}"#);
    ok(
        &mut broker,
        r#"{"Command": "SendMessage", "QueueName": "a", "Body": "x"}"#,
    );
    assert_eq!(counts(&mut broker, "a")["Visible"], 1);
}

#[test]
fn a_queue_holds_at_most_100000_messages() {
    let mut broker = Broker::new();
    standard(&mut broker, "a");
    for _ in 0..100_000 {
        ok(
            &mut broker,
            r#"{"Command": "SendMessage", "QueueName": "a", "Body": "x"}"#,
        );
    }
    fail(
        &mut broker,
        r#"{"Command": "SendMessage", "QueueName": "a", "Body": "x"}"#,
        "LimitExceeded",
    );
    assert_eq!(counts(&mut broker, "a")["Visible"], 100_000);
}
