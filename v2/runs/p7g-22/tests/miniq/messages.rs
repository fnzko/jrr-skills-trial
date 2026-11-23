use serde_json::{Value, json};

use crate::common::*;

fn received(id: &str, handle: &str, body: &str, count: u64, sent_at: u64, first: u64) -> Value {
    json!({"MessageId": id, "ReceiptHandle": handle, "Body": body,
           "ReceiveCount": count, "SentAt": sent_at, "FirstReceivedAt": first})
}

#[test]
fn send_and_receive_round_trip() {
    let responses = run(&[
        create("q"),
        tick(7),
        send("q", "hello"),
        receive("q"),
        receive("q"),
    ]);
    assert_eq!(responses[2], json!({"ok": true, "MessageId": "m-1"}));
    assert_eq!(
        responses[3],
        json!({"ok": true, "Messages": [received("m-1", "r-1", "hello", 1, 7, 7)]})
    );
    assert_eq!(responses[4], json!({"ok": true, "Messages": []}));
}

#[test]
fn message_and_handle_counters_span_queues() {
    let responses = run(&[
        create("a"),
        create("b"),
        send("a", "1"),
        send("b", "2"),
        receive("b"),
        receive("a"),
    ]);
    assert_eq!(responses[3], json!({"ok": true, "MessageId": "m-2"}));
    assert_eq!(responses[4]["Messages"][0]["ReceiptHandle"], json!("r-1"));
    assert_eq!(responses[5]["Messages"][0]["ReceiptHandle"], json!("r-2"));
}

#[test]
fn failed_send_uses_no_message_id() {
    let responses = run(&[
        create("q"),
        send("q", ""),
        json!({"Command": "SendMessage", "QueueName": "q", "Body": "x", "GroupId": "g"}),
        send("q", "ok"),
    ]);
    assert_eq!(responses[3], json!({"ok": true, "MessageId": "m-1"}));
}

#[test]
fn max_messages_and_arrival_order() {
    let responses = run(&[
        create("q"),
        send("q", "a"),
        send("q", "b"),
        send("q", "c"),
        receive_up_to("q", 2),
        receive_up_to("q", 10),
    ]);
    assert_eq!(
        delivered(&responses[4]),
        vec![("m-1".into(), "a".into()), ("m-2".into(), "b".into())]
    );
    assert_eq!(delivered(&responses[5]), vec![("m-3".into(), "c".into())]);
}

#[test]
fn visibility_timeout_ends_and_message_keeps_its_place() {
    let responses = run(&[
        create_with("q", json!({"VisibilityTimeout": 10})),
        send("q", "a"),
        send("q", "b"),
        receive("q"),
        tick(9),
        receive("q"),
        tick(1),
        receive_up_to("q", 2),
    ]);
    assert_eq!(delivered(&responses[5]), vec![("m-2".into(), "b".into())]);
    // m-1's timeout ended at 10 and it is first in arrival order again.
    assert_eq!(delivered(&responses[7]), vec![("m-1".into(), "a".into())]);
    assert_eq!(responses[7]["Messages"][0]["ReceiveCount"], json!(2));
    assert_eq!(responses[7]["Messages"][0]["FirstReceivedAt"], json!(0));
}

#[test]
fn receive_overrides_visibility_timeout() {
    let responses = run(&[
        create("q"),
        send("q", "a"),
        json!({"Command": "ReceiveMessage", "QueueName": "q", "VisibilityTimeout": 2}),
        tick(2),
        receive("q"),
    ]);
    assert_eq!(delivered(&responses[4]), vec![("m-1".into(), "a".into())]);
}

#[test]
fn delay_hides_a_message_until_it_elapses() {
    let responses = run(&[
        create_with("q", json!({"DelaySeconds": 5})),
        send("q", "queue delay"),
        json!({"Command": "SendMessage", "QueueName": "q", "Body": "own delay", "DelaySeconds": 1}),
        json!({"Command": "SendMessage", "QueueName": "q", "Body": "no delay", "DelaySeconds": 0}),
        attributes("q"),
        receive("q"),
        tick(1),
        attributes("q"),
        tick(4),
        receive_up_to("q", 10),
    ]);
    assert_eq!(
        counts(&responses[4]),
        json!({"Visible": 1, "InFlight": 0, "Delayed": 2})
    );
    assert_eq!(
        delivered(&responses[5]),
        vec![("m-3".into(), "no delay".into())]
    );
    assert_eq!(
        counts(&responses[7]),
        json!({"Visible": 1, "InFlight": 1, "Delayed": 1})
    );
    assert_eq!(
        delivered(&responses[9]),
        vec![
            ("m-1".into(), "queue delay".into()),
            ("m-2".into(), "own delay".into())
        ]
    );
}

fn fifo(name: &str) -> Value {
    create_with(name, json!({"Fifo": true}))
}

#[test]
fn fifo_send_validation_follows_rule_order() {
    let base = |extra: Value| {
        let mut request = json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "x"});
        request
            .as_object_mut()
            .expect("literal is an object")
            .extend(extra.as_object().cloned().expect("extra is an object"));
        request
    };
    let cases = [
        (
            base(json!({"DelaySeconds": 0, "GroupId": "g"})),
            "UnsupportedOperation",
        ),
        (base(json!({"DeduplicationId": "d"})), "MissingParameter"),
        (base(json!({"GroupId": "g"})), "MissingParameter"),
    ];
    for (request, expected) in cases {
        let responses = run(&[fifo("f.fifo"), request.clone()]);
        assert_eq!(code(&responses[1]), expected, "{request}");
    }
    let standard = run(&[
        create("s"),
        json!({"Command": "SendMessage", "QueueName": "s", "Body": "x", "DeduplicationId": "d"}),
    ]);
    assert_eq!(code(&standard[1]), "UnsupportedOperation");
}

#[test]
fn fifo_sequence_numbers_count_per_queue() {
    let responses = run(&[
        fifo("a.fifo"),
        fifo("b.fifo"),
        send_fifo("a.fifo", "1", "g", "d1"),
        send_fifo("b.fifo", "2", "g", "d1"),
        send_fifo("a.fifo", "3", "g", "d2"),
    ]);
    assert_eq!(
        responses[2],
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
    assert_eq!(
        responses[3],
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 1})
    );
    assert_eq!(
        responses[4],
        json!({"ok": true, "MessageId": "m-3", "SequenceNumber": 2})
    );
}

#[test]
fn duplicate_sends_return_the_original_and_use_no_id() {
    let responses = run(&[
        fifo("f.fifo"),
        send_fifo("f.fifo", "a", "g", "d"),
        tick(299),
        send_fifo("f.fifo", "other body", "h", "d"),
        send_fifo("f.fifo", "c", "g", "e"),
    ]);
    assert_eq!(
        responses[3],
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
    assert_eq!(
        responses[4],
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2})
    );
}

#[test]
fn deduplication_window_reopens_at_300_and_is_not_extended_by_duplicates() {
    let responses = run(&[
        fifo("f.fifo"),
        send_fifo("f.fifo", "a", "g", "d"),
        tick(200),
        send_fifo("f.fifo", "a", "g", "d"),
        tick(100),
        send_fifo("f.fifo", "a", "g", "d"),
        tick(299),
        send_fifo("f.fifo", "a", "g", "d"),
    ]);
    assert_eq!(responses[3]["MessageId"], json!("m-1"));
    assert_eq!(
        responses[5],
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2})
    );
    assert_eq!(responses[7]["MessageId"], json!("m-2"));
}

#[test]
fn deduplication_outlives_the_message_and_ignores_the_group() {
    let responses = run(&[
        fifo("f.fifo"),
        send_fifo("f.fifo", "a", "g", "d"),
        receive("f.fifo"),
        delete("f.fifo", "r-1"),
        send_fifo("f.fifo", "a", "other-group", "d"),
    ]);
    assert_eq!(responses[3], ok());
    assert_eq!(
        responses[4],
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
}

#[test]
fn content_based_deduplication_uses_the_body() {
    let mut first =
        json!({"Command": "SendMessage", "QueueName": "c.fifo", "Body": "same", "GroupId": "g"});
    let second = first.clone();
    first["DeduplicationId"] = json!("explicit");
    let responses = run(&[
        create_with(
            "c.fifo",
            json!({"Fifo": true, "ContentBasedDeduplication": true}),
        ),
        second.clone(),
        second,
        first,
    ]);
    assert_eq!(responses[2]["MessageId"], json!("m-1"));
    assert_eq!(responses[3]["MessageId"], json!("m-2"));
}

#[test]
fn fifo_delivers_one_message_per_group_and_blocks_busy_groups() {
    let responses = run(&[
        fifo("f.fifo"),
        send_fifo("f.fifo", "a1", "a", "1"),
        send_fifo("f.fifo", "a2", "a", "2"),
        send_fifo("f.fifo", "b1", "b", "3"),
        send_fifo("f.fifo", "b2", "b", "4"),
        receive_up_to("f.fifo", 10),
        receive_up_to("f.fifo", 10),
        delete("f.fifo", "r-1"),
        receive_up_to("f.fifo", 10),
    ]);
    assert_eq!(
        delivered(&responses[5]),
        vec![("m-1".into(), "a1".into()), ("m-3".into(), "b1".into())]
    );
    assert_eq!(delivered(&responses[6]), vec![]);
    assert_eq!(delivered(&responses[8]), vec![("m-2".into(), "a2".into())]);
    assert_eq!(responses[8]["Messages"][0]["GroupId"], json!("a"));
    assert_eq!(responses[8]["Messages"][0]["SequenceNumber"], json!(2));
}

#[test]
fn a_delayed_or_in_flight_head_blocks_its_group_but_not_others() {
    let responses = run(&[
        create_with("f.fifo", json!({"Fifo": true, "DelaySeconds": 10})),
        send_fifo("f.fifo", "a1", "a", "1"),
        tick(10),
        send_fifo("f.fifo", "a2", "a", "2"),
        send_fifo("f.fifo", "b1", "b", "3"),
        set_attributes("f.fifo", json!({"DelaySeconds": 0})),
        receive_up_to("f.fifo", 10),
    ]);
    // a1 is visible at 10, so it is delivered first; a2 and b1 are delayed.
    assert_eq!(delivered(&responses[6]), vec![("m-1".into(), "a1".into())]);
}

#[test]
fn zero_visibility_makes_the_message_visible_at_once_and_its_handle_unusable() {
    let responses = run(&[
        create("q"),
        send("q", "a"),
        json!({"Command": "ReceiveMessage", "QueueName": "q", "VisibilityTimeout": 0}),
        delete("q", "r-1"),
        receive("q"),
    ]);
    assert_eq!(code(&responses[3]), "ReceiptHandleExpired");
    assert_eq!(responses[4]["Messages"][0]["ReceiptHandle"], json!("r-2"));
    assert_eq!(responses[4]["Messages"][0]["ReceiveCount"], json!(2));
}

#[test]
fn delete_and_change_visibility_check_handles() {
    let responses = run(&[
        create("q"),
        create("other"),
        send("q", "a"),
        send("other", "b"),
        receive("q"),
        receive("other"),
        delete("q", "r-01"),
        delete("q", "R-1"),
        delete("q", "r-1 "),
        delete("q", "r-9"),
        delete("q", "r-2"),
        change_visibility("q", "r-1", 5),
        delete("q", "r-1"),
        delete("q", "r-1"),
        attributes("q"),
    ]);
    for response in &responses[6..=9] {
        assert_eq!(code(response), "ReceiptHandleInvalid");
    }
    assert_eq!(code(&responses[10]), "ReceiptHandleExpired");
    assert_eq!(responses[11], ok());
    assert_eq!(responses[12], ok());
    assert_eq!(code(&responses[13]), "ReceiptHandleExpired");
    assert_eq!(
        counts(&responses[14]),
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn only_the_latest_handle_of_a_message_works() {
    let responses = run(&[
        create("q"),
        send("q", "a"),
        json!({"Command": "ReceiveMessage", "QueueName": "q", "VisibilityTimeout": 1}),
        tick(1),
        receive("q"),
        delete("q", "r-1"),
        change_visibility("q", "r-1", 9),
        delete("q", "r-2"),
    ]);
    assert_eq!(code(&responses[5]), "ReceiptHandleExpired");
    assert_eq!(code(&responses[6]), "ReceiptHandleExpired");
    assert_eq!(responses[7], ok());
}

#[test]
fn an_ended_visibility_timeout_expires_the_handle() {
    let responses = run(&[
        create_with("q", json!({"VisibilityTimeout": 5})),
        send("q", "a"),
        receive("q"),
        tick(5),
        change_visibility("q", "r-1", 5),
        delete("q", "r-1"),
    ]);
    assert_eq!(code(&responses[4]), "ReceiptHandleExpired");
    assert_eq!(code(&responses[5]), "ReceiptHandleExpired");
}

#[test]
fn change_visibility_restarts_the_timeout_from_now() {
    let responses = run(&[
        create_with("q", json!({"VisibilityTimeout": 10})),
        send("q", "a"),
        receive("q"),
        tick(8),
        change_visibility("q", "r-1", 5),
        tick(4),
        attributes("q"),
        tick(1),
        attributes("q"),
        receive("q"),
    ]);
    assert_eq!(
        counts(&responses[6]),
        json!({"Visible": 0, "InFlight": 1, "Delayed": 0})
    );
    assert_eq!(
        counts(&responses[8]),
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
    // The count is untouched by the change; this is the second delivery.
    assert_eq!(responses[9]["Messages"][0]["ReceiveCount"], json!(2));
}

#[test]
fn change_visibility_to_zero_makes_the_message_visible_at_once() {
    let responses = run(&[
        create("q"),
        send("q", "a"),
        receive("q"),
        change_visibility("q", "r-1", 0),
        attributes("q"),
    ]);
    assert_eq!(
        counts(&responses[4]),
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn retention_removes_messages_in_every_state() {
    let responses = run(&[
        create_with(
            "q",
            json!({"RetentionSeconds": 60, "VisibilityTimeout": 100}),
        ),
        send("q", "visible"),
        send("q", "taken"),
        json!({"Command": "SendMessage", "QueueName": "q", "Body": "late", "DelaySeconds": 900}),
        receive_up_to("q", 1),
        tick(59),
        attributes("q"),
        tick(1),
        attributes("q"),
        delete("q", "r-1"),
    ]);
    assert_eq!(
        counts(&responses[6]),
        json!({"Visible": 1, "InFlight": 1, "Delayed": 1})
    );
    assert_eq!(
        counts(&responses[8]),
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(code(&responses[9]), "ReceiptHandleExpired");
}

#[test]
fn lowering_retention_removes_at_once_and_raising_it_restores_nothing() {
    let responses = run(&[
        create("q"),
        send("q", "old"),
        tick(100),
        send("q", "new"),
        set_attributes("q", json!({"RetentionSeconds": 100})),
        attributes("q"),
        set_attributes("q", json!({"RetentionSeconds": 1_209_600})),
        attributes("q"),
    ]);
    assert_eq!(
        counts(&responses[5]),
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(
        counts(&responses[7]),
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn raising_retention_after_the_time_has_passed_does_not_resurrect() {
    let responses = run(&[
        create_with("q", json!({"RetentionSeconds": 60})),
        send("q", "gone"),
        tick(60),
        set_attributes("q", json!({"RetentionSeconds": 1000})),
        attributes("q"),
    ]);
    assert_eq!(
        counts(&responses[4]),
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn messages_move_to_the_dead_letter_queue_after_max_receives() {
    let responses = run(&[
        create("dead"),
        create_with(
            "q",
            json!({"VisibilityTimeout": 0, "DeadLetterQueue": "dead", "MaxReceiveCount": 2}),
        ),
        send("q", "poison"),
        send("q", "fine"),
        receive("q"),
        receive("q"),
        receive_up_to("q", 5),
        attributes("q"),
        receive("dead"),
    ]);
    assert_eq!(
        delivered(&responses[4]),
        vec![("m-1".into(), "poison".into())]
    );
    assert_eq!(
        delivered(&responses[5]),
        vec![("m-1".into(), "poison".into())]
    );
    // On the third look poison moves away and does not count toward the max.
    assert_eq!(
        delivered(&responses[6]),
        vec![("m-2".into(), "fine".into())]
    );
    assert_eq!(
        counts(&responses[7]),
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(
        responses[8],
        json!({"ok": true, "Messages": [received("m-1", "r-4", "poison", 1, 0, 0)]})
    );
}

#[test]
fn dead_lettered_messages_are_reset_and_take_the_next_sequence_number() {
    let responses = run(&[
        fifo("dead.fifo"),
        send_fifo("dead.fifo", "existing", "x", "e"),
        create_with(
            "q.fifo",
            json!({"Fifo": true, "VisibilityTimeout": 0,
                   "DeadLetterQueue": "dead.fifo", "MaxReceiveCount": 1}),
        ),
        tick(5),
        send_fifo("q.fifo", "poison", "g", "d"),
        receive("q.fifo"),
        receive("q.fifo"),
        receive_up_to("dead.fifo", 10),
    ]);
    assert_eq!(delivered(&responses[6]), vec![]);
    assert_eq!(
        responses[7]["Messages"],
        json!([
            {"MessageId": "m-1", "ReceiptHandle": "r-2", "Body": "existing", "ReceiveCount": 1,
             "SentAt": 0, "FirstReceivedAt": 5, "GroupId": "x", "SequenceNumber": 1},
            {"MessageId": "m-2", "ReceiptHandle": "r-3", "Body": "poison", "ReceiveCount": 1,
             "SentAt": 5, "FirstReceivedAt": 5, "GroupId": "g", "SequenceNumber": 2},
        ])
    );
}

#[test]
fn a_message_too_old_for_the_dead_letter_queue_is_removed_on_arrival() {
    let responses = run(&[
        create_with("dead", json!({"RetentionSeconds": 60})),
        create_with(
            "q",
            json!({"VisibilityTimeout": 0, "DeadLetterQueue": "dead", "MaxReceiveCount": 1}),
        ),
        send("q", "a"),
        receive("q"),
        tick(60),
        receive("q"),
        attributes("dead"),
    ]);
    assert_eq!(delivered(&responses[5]), vec![]);
    assert_eq!(
        counts(&responses[6]),
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn a_new_dead_letter_configuration_applies_from_the_next_receive() {
    let responses = run(&[
        create("dead"),
        create_with("q", json!({"VisibilityTimeout": 0})),
        send("q", "a"),
        receive("q"),
        receive("q"),
        set_attributes(
            "q",
            json!({"DeadLetterQueue": "dead", "MaxReceiveCount": 2}),
        ),
        receive("q"),
        attributes("dead"),
    ]);
    assert_eq!(delivered(&responses[6]), vec![]);
    assert_eq!(
        counts(&responses[7]),
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
}
