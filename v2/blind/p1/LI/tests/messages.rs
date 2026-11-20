mod common;

use common::{Session, bodies, error_code, success};
use serde_json::{Value, json};

fn delete(s: &mut Session, queue: &str, handle: &str) -> Value {
    s.run(json!({"Command": "DeleteMessage", "QueueName": queue, "ReceiptHandle": handle}))
}

fn change(s: &mut Session, queue: &str, handle: &str, timeout: u32) -> Value {
    s.run(json!({"Command": "ChangeMessageVisibility", "QueueName": queue, "ReceiptHandle": handle, "VisibilityTimeout": timeout}))
}

fn counts(visible: u32, in_flight: u32, delayed: u32) -> Value {
    json!({"Visible": visible, "InFlight": in_flight, "Delayed": delayed})
}

#[test]
fn message_ids_count_accepted_messages_over_all_queues() {
    let mut s = Session::new();
    s.create("a", json!({}));
    s.create("b", json!({}));
    let failed = s.run(json!({"Command": "SendMessage", "QueueName": "ghost", "Body": "x"}));
    assert_eq!(error_code(&failed), "QueueDoesNotExist");
    assert_eq!(s.send("a", "x"), json!({"ok": true, "MessageId": "m-1"}));
    assert_eq!(s.send("b", "x"), json!({"ok": true, "MessageId": "m-2"}));
    assert_eq!(s.send("a", "x"), json!({"ok": true, "MessageId": "m-3"}));
}

#[test]
fn receive_returns_the_documented_fields() {
    let mut s = Session::new();
    s.create("q", json!({}));
    s.tick(5);
    s.send("q", "héllo");
    s.tick(2);
    assert_eq!(
        s.receive("q"),
        json!({"ok": true, "Messages": [{
            "MessageId": "m-1",
            "ReceiptHandle": "r-1",
            "Body": "héllo",
            "ReceiveCount": 1,
            "SentAt": 5,
            "FirstReceivedAt": 7,
        }]})
    );
    assert_eq!(s.receive("q"), json!({"ok": true, "Messages": []}));
}

#[test]
fn redelivery_keeps_arrival_order_and_first_received_time() {
    let mut s = Session::new();
    s.create("q", json!({}));
    s.send("q", "a");
    s.receive("q");
    s.send("q", "b");
    s.send("q", "c");
    s.tick(30);
    let response = s.ok(json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 10}));
    assert_eq!(bodies(&response), ["a", "b", "c"]);
    let messages = &response["Messages"];
    assert_eq!(messages[0]["ReceiveCount"], json!(2));
    assert_eq!(messages[0]["FirstReceivedAt"], json!(0));
    assert_eq!(messages[1]["ReceiveCount"], json!(1));
    assert_eq!(messages[1]["FirstReceivedAt"], json!(30));
    // Handles are numbered in delivery order, over all deliveries.
    let handles: Vec<_> = messages
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["ReceiptHandle"].clone())
        .collect();
    assert_eq!(handles, [json!("r-2"), json!("r-3"), json!("r-4")]);
}

#[test]
fn max_messages_limits_a_receive() {
    let mut s = Session::new();
    s.create("q", json!({}));
    for body in ["a", "b", "c"] {
        s.send("q", body);
    }
    let two = s.ok(json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 2}));
    assert_eq!(bodies(&two), ["a", "b"]);
    assert_eq!(bodies(&s.receive("q")), ["c"]);
    for bad in [0, 11] {
        let response =
            s.run(json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": bad}));
        assert_eq!(error_code(&response), "InvalidParameterValue");
    }
}

#[test]
fn delays_hide_messages_until_they_pass() {
    let mut s = Session::new();
    s.create("q", json!({"DelaySeconds": 10}));
    s.send("q", "queue default");
    s.ok(
        json!({"Command": "SendMessage", "QueueName": "q", "Body": "immediate", "DelaySeconds": 0}),
    );
    s.ok(json!({"Command": "SendMessage", "QueueName": "q", "Body": "longer", "DelaySeconds": 20}));
    assert_eq!(s.counts("q"), counts(1, 0, 2));
    assert_eq!(bodies(&s.receive("q")), ["immediate"]);
    s.tick(9);
    assert_eq!(s.counts("q"), counts(0, 1, 2));
    s.tick(1);
    assert_eq!(s.counts("q"), counts(1, 1, 1));
    assert_eq!(bodies(&s.receive("q")), ["queue default"]);
    s.tick(10);
    assert_eq!(bodies(&s.receive("q")), ["longer"]);
}

#[test]
fn new_queue_attributes_apply_to_later_operations_only() {
    let mut s = Session::new();
    s.create("q", json!({"DelaySeconds": 10, "VisibilityTimeout": 10}));
    s.send("q", "before");
    s.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"DelaySeconds": 0, "VisibilityTimeout": 100}}));
    s.send("q", "after");
    s.tick(10);
    assert_eq!(
        bodies(&s.ok(json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 2}))),
        ["before", "after"]
    );
    s.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"VisibilityTimeout": 1}}));
    // The deliveries made with the old timeout of 100 keep it.
    s.tick(99);
    assert_eq!(s.counts("q"), counts(0, 2, 0));
    s.tick(1);
    assert_eq!(s.counts("q"), counts(2, 0, 0));
}

#[test]
fn receive_visibility_timeout_overrides_the_queue_attribute() {
    let mut s = Session::new();
    s.create("q", json!({}));
    s.send("q", "a");
    s.ok(json!({"Command": "ReceiveMessage", "QueueName": "q", "VisibilityTimeout": 5}));
    s.tick(4);
    assert_eq!(s.counts("q"), counts(0, 1, 0));
    s.tick(1);
    assert_eq!(s.counts("q"), counts(1, 0, 0));
}

#[test]
fn a_zero_visibility_timeout_makes_the_handle_useless() {
    let mut s = Session::new();
    s.create("q", json!({"VisibilityTimeout": 0}));
    s.send("q", "a");
    assert_eq!(s.receive("q")["Messages"][0]["ReceiptHandle"], json!("r-1"));
    assert_eq!(
        error_code(&delete(&mut s, "q", "r-1")),
        "ReceiptHandleExpired"
    );
    assert_eq!(s.counts("q"), counts(1, 0, 0));
    assert_eq!(s.receive("q")["Messages"][0]["ReceiveCount"], json!(2));
}

#[test]
fn delete_message_handle_checks() {
    let mut s = Session::new();
    s.create("q", json!({}));
    s.create("other", json!({}));
    assert_eq!(
        error_code(&delete(&mut s, "q", "r-1")),
        "ReceiptHandleInvalid"
    );
    s.send("q", "a");
    s.send("other", "b");
    s.receive("q");
    s.receive("other");
    for never_issued in ["r-01", "R-1", "r-1 ", "r-3", "r-0", "1", ""] {
        assert_eq!(
            error_code(&delete(&mut s, "q", never_issued)),
            "ReceiptHandleInvalid",
            "{never_issued:?}"
        );
    }
    // r-2 was issued for a message of another queue.
    assert_eq!(
        error_code(&delete(&mut s, "q", "r-2")),
        "ReceiptHandleExpired"
    );
    assert_eq!(delete(&mut s, "q", "r-1"), success());
    assert_eq!(s.counts("q"), counts(0, 0, 0));
    assert_eq!(
        error_code(&delete(&mut s, "q", "r-1")),
        "ReceiptHandleExpired"
    );
}

#[test]
fn delete_message_checks_the_queue_before_the_handle() {
    let mut s = Session::new();
    assert_eq!(
        error_code(&delete(&mut s, "ghost", "bogus")),
        "QueueDoesNotExist"
    );
    s.create("q", json!({}));
    let response = s.run(
        json!({"Command": "ChangeMessageVisibility", "QueueName": "q", "ReceiptHandle": "bogus"}),
    );
    assert_eq!(error_code(&response), "MissingParameter");
    assert_eq!(
        error_code(&change(&mut s, "q", "r-1", 43201)),
        "InvalidParameterValue"
    );
}

#[test]
fn only_the_latest_handle_of_an_in_flight_message_works() {
    let mut s = Session::new();
    s.create("q", json!({}));
    s.send("q", "a");
    s.receive("q"); // r-1, in flight until 30
    s.tick(29);
    assert_eq!(change(&mut s, "q", "r-1", 5), success()); // now until 34
    s.tick(4);
    assert_eq!(s.counts("q"), counts(0, 1, 0));
    s.tick(1);
    assert_eq!(s.counts("q"), counts(1, 0, 0));
    assert_eq!(
        error_code(&delete(&mut s, "q", "r-1")),
        "ReceiptHandleExpired"
    );
    assert_eq!(
        error_code(&change(&mut s, "q", "r-1", 5)),
        "ReceiptHandleExpired"
    );
    assert_eq!(s.receive("q")["Messages"][0]["ReceiptHandle"], json!("r-2"));
    assert_eq!(
        error_code(&delete(&mut s, "q", "r-1")),
        "ReceiptHandleExpired"
    );
    assert_eq!(delete(&mut s, "q", "r-2"), success());
}

#[test]
fn change_message_visibility_to_zero_releases_the_message_without_a_new_receive_count() {
    let mut s = Session::new();
    s.create("q", json!({}));
    s.send("q", "a");
    s.receive("q");
    assert_eq!(change(&mut s, "q", "r-1", 0), success());
    assert_eq!(s.counts("q"), counts(1, 0, 0));
    assert_eq!(
        error_code(&delete(&mut s, "q", "r-1")),
        "ReceiptHandleExpired"
    );
    assert_eq!(s.receive("q")["Messages"][0]["ReceiveCount"], json!(2));
}

#[test]
fn change_message_visibility_can_extend_beyond_the_original_timeout() {
    let mut s = Session::new();
    s.create("q", json!({}));
    s.send("q", "a");
    s.receive("q");
    assert_eq!(change(&mut s, "q", "r-1", 43200), success());
    s.tick(43199);
    assert_eq!(s.counts("q"), counts(0, 1, 0));
    assert_eq!(delete(&mut s, "q", "r-1"), success());
}

#[test]
fn standard_queue_rejects_fifo_parameters_and_fifo_queue_rejects_delay() {
    let mut s = Session::new();
    s.create("std", json!({}));
    s.create("f.fifo", json!({"Fifo": true}));
    let send = |s: &mut Session, queue: &str, extra: Value| {
        let mut command = json!({"Command": "SendMessage", "QueueName": queue, "Body": "x"});
        command
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        error_code(&s.run(command)).to_owned()
    };
    assert_eq!(
        send(&mut s, "std", json!({"GroupId": "g"})),
        "UnsupportedOperation"
    );
    assert_eq!(
        send(&mut s, "std", json!({"DeduplicationId": "d"})),
        "UnsupportedOperation"
    );
    assert_eq!(
        send(
            &mut s,
            "f.fifo",
            json!({"DelaySeconds": 0, "GroupId": "g", "DeduplicationId": "d"})
        ),
        "UnsupportedOperation"
    );
    // Unsupported wins over missing.
    assert_eq!(
        send(&mut s, "f.fifo", json!({"DelaySeconds": 1})),
        "UnsupportedOperation"
    );
    assert_eq!(
        send(&mut s, "f.fifo", json!({"DeduplicationId": "d"})),
        "MissingParameter"
    );
    assert_eq!(
        send(&mut s, "f.fifo", json!({"GroupId": "g"})),
        "MissingParameter"
    );
    assert_eq!(s.counts("std"), counts(0, 0, 0));
    assert_eq!(s.counts("f.fifo"), counts(0, 0, 0));
}

fn send_fifo(s: &mut Session, queue: &str, body: &str, group: &str, dedup: &str) -> Value {
    s.ok(json!({"Command": "SendMessage", "QueueName": queue, "Body": body, "GroupId": group, "DeduplicationId": dedup}))
}

#[test]
fn fifo_sequence_numbers_count_per_queue() {
    let mut s = Session::new();
    s.create("a.fifo", json!({"Fifo": true}));
    s.create("b.fifo", json!({"Fifo": true}));
    assert_eq!(
        send_fifo(&mut s, "a.fifo", "x", "g", "1"),
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
    assert_eq!(
        send_fifo(&mut s, "b.fifo", "x", "g", "1"),
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 1})
    );
    assert_eq!(
        send_fifo(&mut s, "a.fifo", "x", "g", "2"),
        json!({"ok": true, "MessageId": "m-3", "SequenceNumber": 2})
    );
    let received = s.receive("a.fifo");
    assert_eq!(received["Messages"][0]["GroupId"], json!("g"));
    assert_eq!(received["Messages"][0]["SequenceNumber"], json!(1));
}

#[test]
fn deduplication_window_is_300_seconds_and_a_duplicate_does_not_extend_it() {
    let mut s = Session::new();
    s.create("f.fifo", json!({"Fifo": true}));
    let first = send_fifo(&mut s, "f.fifo", "one", "g", "d");
    assert_eq!(
        first,
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
    s.tick(299);
    // A duplicate returns the original ids, whatever its body and group.
    assert_eq!(send_fifo(&mut s, "f.fifo", "two", "other", "d"), first);
    s.tick(1);
    let reopened = send_fifo(&mut s, "f.fifo", "three", "g", "d");
    assert_eq!(
        reopened,
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2})
    );
    s.tick(299);
    assert_eq!(send_fifo(&mut s, "f.fifo", "four", "g", "d"), reopened);
    s.tick(1);
    assert_eq!(
        send_fifo(&mut s, "f.fifo", "five", "g", "d")["MessageId"],
        json!("m-3")
    );
    assert_eq!(s.counts("f.fifo"), counts(3, 0, 0));
}

#[test]
fn deduplication_outlives_the_message_and_belongs_to_one_queue() {
    let mut s = Session::new();
    s.create("a.fifo", json!({"Fifo": true}));
    s.create("b.fifo", json!({"Fifo": true}));
    let first = send_fifo(&mut s, "a.fifo", "x", "g", "d");
    let handle = s.receive("a.fifo")["Messages"][0]["ReceiptHandle"].clone();
    assert_eq!(
        delete(&mut s, "a.fifo", handle.as_str().unwrap()),
        success()
    );
    assert_eq!(send_fifo(&mut s, "a.fifo", "x", "g", "d"), first);
    s.ok(json!({"Command": "PurgeQueue", "QueueName": "a.fifo"}));
    assert_eq!(send_fifo(&mut s, "a.fifo", "x", "g", "d"), first);
    assert_eq!(s.counts("a.fifo"), counts(0, 0, 0));
    assert_eq!(
        send_fifo(&mut s, "b.fifo", "x", "g", "d")["MessageId"],
        json!("m-2")
    );
    // Purging does not restart sequence numbers.
    assert_eq!(
        send_fifo(&mut s, "a.fifo", "x", "g", "e")["SequenceNumber"],
        json!(2)
    );
}

#[test]
fn content_based_deduplication_uses_the_body_unless_an_id_is_given() {
    let mut s = Session::new();
    s.create(
        "f.fifo",
        json!({"Fifo": true, "ContentBasedDeduplication": true}),
    );
    let send = |s: &mut Session, body: &str, extra: Value| {
        let mut command =
            json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": body, "GroupId": "g"});
        command
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        s.ok(command)["MessageId"].clone()
    };
    let long_body = "é".repeat(512);
    assert_eq!(send(&mut s, "same", json!({})), json!("m-1"));
    assert_eq!(send(&mut s, "same", json!({})), json!("m-1"));
    assert_eq!(
        send(&mut s, "same", json!({"DeduplicationId": "explicit"})),
        json!("m-2")
    );
    assert_eq!(send(&mut s, &long_body, json!({})), json!("m-3"));
    assert_eq!(send(&mut s, &long_body, json!({})), json!("m-3"));
}

#[test]
fn fifo_receive_delivers_one_message_per_group_in_arrival_order() {
    let mut s = Session::new();
    s.create("f.fifo", json!({"Fifo": true}));
    send_fifo(&mut s, "f.fifo", "a1", "a", "1");
    send_fifo(&mut s, "f.fifo", "b1", "b", "2");
    send_fifo(&mut s, "f.fifo", "a2", "a", "3");
    let first =
        s.ok(json!({"Command": "ReceiveMessage", "QueueName": "f.fifo", "MaxMessages": 10}));
    assert_eq!(bodies(&first), ["a1", "b1"]);
    // Both groups are blocked while their earliest message is in flight.
    assert_eq!(
        bodies(
            &s.ok(json!({"Command": "ReceiveMessage", "QueueName": "f.fifo", "MaxMessages": 10}))
        ),
        Vec::<&str>::new()
    );
    let a1_handle = first["Messages"][0]["ReceiptHandle"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(delete(&mut s, "f.fifo", &a1_handle), success());
    // a2 is next in group a, but b1 is still in flight.
    let next = s.ok(json!({"Command": "ReceiveMessage", "QueueName": "f.fifo", "MaxMessages": 10}));
    assert_eq!(bodies(&next), ["a2"]);
    assert_eq!(next["Messages"][0]["SequenceNumber"], json!(3));
}

#[test]
fn fifo_group_stays_blocked_while_its_earliest_message_is_delayed() {
    let mut s = Session::new();
    s.create("f.fifo", json!({"Fifo": true, "DelaySeconds": 10}));
    send_fifo(&mut s, "f.fifo", "a1", "a", "1");
    s.ok(json!({"Command": "SetQueueAttributes", "QueueName": "f.fifo", "Attributes": {"DelaySeconds": 0}}));
    send_fifo(&mut s, "f.fifo", "a2", "a", "2");
    send_fifo(&mut s, "f.fifo", "b1", "b", "3");
    assert_eq!(bodies(&s.receive("f.fifo")), ["b1"]);
    s.tick(10);
    let response =
        s.ok(json!({"Command": "ReceiveMessage", "QueueName": "f.fifo", "MaxMessages": 10}));
    assert_eq!(bodies(&response), ["a1"]);
}

#[test]
fn messages_over_the_dead_letter_limit_move_to_the_dead_letter_queue() {
    let mut s = Session::new();
    s.create("dead", json!({}));
    s.create(
        "q",
        json!({"DeadLetterQueue": "dead", "MaxReceiveCount": 2, "VisibilityTimeout": 10}),
    );
    s.tick(3);
    s.send("q", "x");
    s.receive("q");
    s.tick(10);
    s.receive("q");
    s.tick(10);
    assert_eq!(s.receive("q"), json!({"ok": true, "Messages": []}));
    assert_eq!(s.counts("q"), counts(0, 0, 0));
    assert_eq!(s.counts("dead"), counts(1, 0, 0));
    assert_eq!(
        s.receive("dead"),
        json!({"ok": true, "Messages": [{
            "MessageId": "m-1",
            "ReceiptHandle": "r-3",
            "Body": "x",
            "ReceiveCount": 1,
            "SentAt": 3,
            "FirstReceivedAt": 23,
        }]})
    );
}

#[test]
fn dead_letter_moves_do_not_count_toward_max_messages() {
    let mut s = Session::new();
    s.create("dead", json!({}));
    s.create(
        "q",
        json!({"DeadLetterQueue": "dead", "MaxReceiveCount": 1}),
    );
    s.send("q", "a");
    s.send("q", "b");
    assert_eq!(bodies(&s.receive("q")), ["a"]);
    s.tick(30);
    assert_eq!(bodies(&s.receive("q")), ["b"]);
    assert_eq!(s.counts("dead"), counts(1, 0, 0));
}

#[test]
fn dead_letter_move_applies_the_target_retention_to_the_original_send_time() {
    let mut s = Session::new();
    s.create("dead", json!({"RetentionSeconds": 60}));
    s.create(
        "q",
        json!({"DeadLetterQueue": "dead", "MaxReceiveCount": 1, "VisibilityTimeout": 100}),
    );
    s.send("q", "old");
    s.receive("q");
    s.tick(100);
    s.receive("q");
    assert_eq!(s.counts("dead"), counts(0, 0, 0));
}

#[test]
fn dead_letter_move_in_fifo_queue_unblocks_the_group_and_takes_a_new_sequence_number() {
    let mut s = Session::new();
    s.create("d.fifo", json!({"Fifo": true}));
    s.create(
        "m.fifo",
        json!({"Fifo": true, "DeadLetterQueue": "d.fifo", "MaxReceiveCount": 1}),
    );
    send_fifo(&mut s, "d.fifo", "existing", "z", "0");
    send_fifo(&mut s, "m.fifo", "x", "g", "1");
    send_fifo(&mut s, "m.fifo", "y", "g", "2");
    assert_eq!(bodies(&s.receive("m.fifo")), ["x"]);
    s.tick(30);
    let response = s.receive("m.fifo");
    assert_eq!(bodies(&response), ["y"]);
    let moved =
        s.ok(json!({"Command": "ReceiveMessage", "QueueName": "d.fifo", "MaxMessages": 10}));
    assert_eq!(bodies(&moved), ["existing", "x"]);
    assert_eq!(moved["Messages"][1]["SequenceNumber"], json!(2));
    assert_eq!(moved["Messages"][1]["GroupId"], json!("g"));
    assert_eq!(moved["Messages"][1]["MessageId"], json!("m-2"));
}

#[test]
fn changed_dead_letter_configuration_applies_from_the_next_receive() {
    let mut s = Session::new();
    s.create("dead", json!({}));
    s.create("q", json!({}));
    s.send("q", "a");
    s.receive("q");
    s.tick(30);
    s.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"DeadLetterQueue": "dead", "MaxReceiveCount": 1}}));
    assert_eq!(s.counts("q"), counts(1, 0, 0));
    assert_eq!(s.receive("q"), json!({"ok": true, "Messages": []}));
    assert_eq!(s.counts("dead"), counts(1, 0, 0));
}

#[test]
fn a_queue_holds_at_most_100000_messages_in_any_state() {
    let mut s = Session::new();
    s.create("q", json!({"RetentionSeconds": 60}));
    for n in 0..100_000 {
        let response = s.run(json!({"Command": "SendMessage", "QueueName": "q", "Body": "x"}));
        assert_eq!(response["ok"], json!(true), "send {n}: {response}");
    }
    let response = s.run(json!({"Command": "SendMessage", "QueueName": "q", "Body": "x"}));
    assert_eq!(error_code(&response), "LimitExceeded");
    // The failed send used no message id.
    s.tick(60);
    assert_eq!(s.send("q", "x")["MessageId"], json!("m-100001"));
}
