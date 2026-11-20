mod common;

use common::{Client, members};
use serde_json::{Value, json};

#[test]
fn send_and_receive_report_every_member() {
    let mut client = Client::default();
    client.create("jobs", json!({}));
    client.tick(4);
    let response =
        client.call(json!({"Command": "SendMessage", "QueueName": "jobs", "Body": "hello"}));
    assert_eq!(response, json!({"ok": true, "MessageId": "m-1"}));
    client.tick(3);
    let response = client.call(json!({"Command": "ReceiveMessage", "QueueName": "jobs"}));
    assert_eq!(
        response,
        json!({"ok": true, "Messages": [{
            "MessageId": "m-1",
            "ReceiptHandle": "r-1",
            "Body": "hello",
            "ReceiveCount": 1,
            "SentAt": 4,
            "FirstReceivedAt": 7
        }]})
    );
    let response = client.call(json!({"Command": "ReceiveMessage", "QueueName": "jobs"}));
    assert_eq!(response, json!({"ok": true, "Messages": []}));
}

#[test]
fn ids_count_over_all_queues_and_skip_failed_sends() {
    let mut client = Client::default();
    client.create("a", json!({}));
    client.create("b", json!({}));
    assert_eq!(client.send("a", "1"), "m-1");
    let code = client
        .error(json!({"Command": "SendMessage", "QueueName": "a", "Body": "x", "GroupId": "g"}));
    assert_eq!(code, "UnsupportedOperation");
    assert_eq!(client.send("b", "2"), "m-2");
    assert_eq!(members(&client.receive("b", 1), "ReceiptHandle"), ["r-1"]);
    assert_eq!(members(&client.receive("a", 1), "ReceiptHandle"), ["r-2"]);
}

#[test]
fn visibility_timeout_hides_a_message_until_it_ends() {
    let mut client = Client::default();
    client.create("jobs", json!({"VisibilityTimeout": 10}));
    client.send("jobs", "a");
    client.send("jobs", "b");
    assert_eq!(members(&client.receive("jobs", 1), "Body"), ["a"]);
    assert_eq!(client.counts("jobs"), [1, 1, 0]);
    client.tick(9);
    assert_eq!(members(&client.receive("jobs", 10), "Body"), ["b"]);
    client.tick(1);
    // "a" keeps its place ahead of "b" in arrival order.
    let messages = client.receive("jobs", 10);
    assert_eq!(members(&messages, "Body"), ["a"]);
    assert_eq!(members(&messages, "ReceiveCount"), ["2"]);
    assert_eq!(members(&messages, "FirstReceivedAt"), ["0"]);
}

#[test]
fn receive_visibility_timeout_parameter_overrides_the_queue() {
    let mut client = Client::default();
    client.create("jobs", json!({"VisibilityTimeout": 100}));
    client.send("jobs", "a");
    let response = client
        .call(json!({"Command": "ReceiveMessage", "QueueName": "jobs", "VisibilityTimeout": 2}));
    assert_eq!(
        members(response["Messages"].as_array().unwrap(), "Body"),
        ["a"]
    );
    client.tick(2);
    assert_eq!(client.counts("jobs"), [1, 0, 0]);
}

#[test]
fn max_messages_limits_one_receive() {
    let mut client = Client::default();
    client.create("jobs", json!({}));
    for body in ["1", "2", "3"] {
        client.send("jobs", body);
    }
    assert_eq!(members(&client.receive("jobs", 2), "Body"), ["1", "2"]);
    assert_eq!(members(&client.receive("jobs", 10), "Body"), ["3"]);
}

#[test]
fn delay_hides_a_new_message() {
    let mut client = Client::default();
    client.create("jobs", json!({"DelaySeconds": 5}));
    client.send("jobs", "queue delay");
    let response = client.call(json!({"Command": "SendMessage", "QueueName": "jobs", "Body": "own delay", "DelaySeconds": 0}));
    assert_eq!(response, json!({"ok": true, "MessageId": "m-2"}));
    assert_eq!(client.counts("jobs"), [1, 0, 1]);
    assert_eq!(members(&client.receive("jobs", 10), "Body"), ["own delay"]);
    client.tick(5);
    assert_eq!(
        members(&client.receive("jobs", 10), "Body"),
        ["queue delay"]
    );
}

#[test]
fn new_delay_applies_to_later_sends_only() {
    let mut client = Client::default();
    client.create("jobs", json!({}));
    client.send("jobs", "before");
    client.call(json!({"Command": "SetQueueAttributes", "QueueName": "jobs", "Attributes": {"DelaySeconds": 60}}));
    client.send("jobs", "after");
    assert_eq!(client.counts("jobs"), [1, 0, 1]);
}

#[test]
fn delete_message_needs_the_latest_handle_of_a_message_in_flight() {
    let mut client = Client::default();
    client.create("jobs", json!({"VisibilityTimeout": 5}));
    client.create("other", json!({}));
    client.send("jobs", "a");
    client.receive("jobs", 1);
    client.tick(5);
    client.receive("jobs", 1);
    let delete = |handle: &str, queue: &str| json!({"Command": "DeleteMessage", "QueueName": queue, "ReceiptHandle": handle});
    let cases = [
        (delete("r-3", "jobs"), "ReceiptHandleInvalid"),
        (delete("r-02", "jobs"), "ReceiptHandleInvalid"),
        (delete("R-2", "jobs"), "ReceiptHandleInvalid"),
        (delete("r-2 ", "jobs"), "ReceiptHandleInvalid"),
        (delete("r-1", "jobs"), "ReceiptHandleExpired"),
        (delete("r-2", "other"), "ReceiptHandleExpired"),
        (delete("r-2", "nope"), "QueueDoesNotExist"),
    ];
    for (request, want) in cases {
        assert_eq!(client.error(request.clone()), want, "request {request}");
    }
    assert_eq!(client.call(delete("r-2", "jobs")), json!({"ok": true}));
    assert_eq!(client.counts("jobs"), [0, 0, 0]);
    assert_eq!(client.error(delete("r-2", "jobs")), "ReceiptHandleExpired");
}

#[test]
fn handle_expires_when_the_visibility_timeout_ends() {
    let mut client = Client::default();
    client.create("jobs", json!({"VisibilityTimeout": 5}));
    client.send("jobs", "a");
    client.receive("jobs", 1);
    client.tick(5);
    let code = client
        .error(json!({"Command": "DeleteMessage", "QueueName": "jobs", "ReceiptHandle": "r-1"}));
    assert_eq!(code, "ReceiptHandleExpired");
}

#[test]
fn zero_visibility_timeout_handle_is_never_usable() {
    let mut client = Client::default();
    client.create("jobs", json!({"VisibilityTimeout": 0}));
    client.send("jobs", "a");
    assert_eq!(
        members(&client.receive("jobs", 1), "ReceiptHandle"),
        ["r-1"]
    );
    assert_eq!(client.counts("jobs"), [1, 0, 0]);
    let code = client
        .error(json!({"Command": "DeleteMessage", "QueueName": "jobs", "ReceiptHandle": "r-1"}));
    assert_eq!(code, "ReceiptHandleExpired");
}

#[test]
fn change_message_visibility_restarts_the_timeout_from_now() {
    let mut client = Client::default();
    client.create("jobs", json!({"VisibilityTimeout": 5}));
    client.send("jobs", "a");
    client.receive("jobs", 1);
    client.tick(3);
    let change = |timeout: u32| json!({"Command": "ChangeMessageVisibility", "QueueName": "jobs", "ReceiptHandle": "r-1", "VisibilityTimeout": timeout});
    assert_eq!(client.call(change(10)), json!({"ok": true}));
    client.tick(9);
    assert_eq!(client.counts("jobs"), [0, 1, 0]);
    assert_eq!(client.call(change(0)), json!({"ok": true}));
    assert_eq!(client.counts("jobs"), [1, 0, 0]);
    assert_eq!(client.error(change(1)), "ReceiptHandleExpired");
    // The receive count stays as the deliveries made it.
    assert_eq!(members(&client.receive("jobs", 1), "ReceiveCount"), ["2"]);
}

#[test]
fn change_message_visibility_requires_a_timeout() {
    let mut client = Client::default();
    client.create("jobs", json!({}));
    let code = client.error(
        json!({"Command": "ChangeMessageVisibility", "QueueName": "nope", "ReceiptHandle": "r-1"}),
    );
    assert_eq!(code, "MissingParameter");
}

#[test]
fn retention_removes_messages_in_every_state() {
    let mut client = Client::default();
    client.create(
        "jobs",
        json!({"RetentionSeconds": 60, "VisibilityTimeout": 100}),
    );
    client.send("jobs", "in flight");
    client.receive("jobs", 1);
    client.tick(30);
    client.send("jobs", "visible");
    client.call(json!({"Command": "SendMessage", "QueueName": "jobs", "Body": "delayed", "DelaySeconds": 900}));
    assert_eq!(client.counts("jobs"), [1, 1, 1]);
    client.tick(30);
    assert_eq!(client.counts("jobs"), [1, 0, 1]);
    let code = client
        .error(json!({"Command": "DeleteMessage", "QueueName": "jobs", "ReceiptHandle": "r-1"}));
    assert_eq!(code, "ReceiptHandleExpired");
    client.tick(30);
    assert_eq!(client.counts("jobs"), [0, 0, 0]);
}

#[test]
fn lowering_retention_removes_messages_for_good() {
    let mut client = Client::default();
    client.create("jobs", json!({}));
    client.send("jobs", "old");
    client.tick(100);
    client.send("jobs", "new");
    let set = |seconds: u32| json!({"Command": "SetQueueAttributes", "QueueName": "jobs", "Attributes": {"RetentionSeconds": seconds}});
    assert_eq!(client.call(set(60)), json!({"ok": true}));
    assert_eq!(client.counts("jobs"), [1, 0, 0]);
    client.call(set(1000));
    assert_eq!(members(&client.receive("jobs", 10), "Body"), ["new"]);
}

#[test]
fn retention_ended_before_it_was_raised_still_removes() {
    let mut client = Client::default();
    client.create("jobs", json!({"RetentionSeconds": 60}));
    client.send("jobs", "old");
    client.tick(60);
    client.call(json!({"Command": "SetQueueAttributes", "QueueName": "jobs", "Attributes": {"RetentionSeconds": 1000}}));
    assert_eq!(client.counts("jobs"), [0, 0, 0]);
}

#[test]
fn purge_removes_all_messages_but_keeps_sequence_and_deduplication() {
    let mut client = Client::default();
    client.create("q.fifo", json!({"Fifo": true}));
    let send = |dedup: &str| json!({"Command": "SendMessage", "QueueName": "q.fifo", "Body": "b", "GroupId": "g", "DeduplicationId": dedup});
    client.call(send("one"));
    client.receive("q.fifo", 1);
    assert_eq!(
        client.call(json!({"Command": "PurgeQueue", "QueueName": "q.fifo"})),
        json!({"ok": true})
    );
    assert_eq!(client.counts("q.fifo"), [0, 0, 0]);
    assert_eq!(
        client.call(send("one")),
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
    assert_eq!(
        client.call(send("two")),
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2})
    );
}

#[test]
fn send_message_rule_order() {
    let mut client = Client::default();
    client.create("std", json!({}));
    client.create("f.fifo", json!({"Fifo": true}));
    let cases = [
        (
            json!({"QueueName": "std", "Body": "b", "DeduplicationId": "d"}),
            "UnsupportedOperation",
        ),
        (
            json!({"QueueName": "f.fifo", "Body": "b", "DelaySeconds": 0}),
            "UnsupportedOperation",
        ),
        (
            json!({"QueueName": "f.fifo", "Body": "b"}),
            "MissingParameter",
        ),
        (
            json!({"QueueName": "f.fifo", "Body": "b", "GroupId": "g"}),
            "MissingParameter",
        ),
        (
            json!({"QueueName": "nope", "Body": ""}),
            "InvalidParameterValue",
        ),
        (
            json!({"QueueName": "nope", "Body": "b"}),
            "QueueDoesNotExist",
        ),
        (json!({"QueueName": "std"}), "MissingParameter"),
    ];
    for (mut request, want) in cases {
        request["Command"] = json!("SendMessage");
        assert_eq!(client.error(request.clone()), want, "request {request}");
    }
}

#[test]
fn fifo_delivers_one_message_per_group_in_order() {
    let mut client = Client::default();
    client.create(
        "q.fifo",
        json!({"Fifo": true, "ContentBasedDeduplication": true}),
    );
    for (body, group) in [
        ("a1", "a"),
        ("b1", "b"),
        ("a2", "a"),
        ("c1", "c"),
        ("b2", "b"),
    ] {
        client.call(json!({"Command": "SendMessage", "QueueName": "q.fifo", "Body": body, "GroupId": group}));
    }
    let messages = client.receive("q.fifo", 10);
    assert_eq!(members(&messages, "Body"), ["a1", "b1", "c1"]);
    assert_eq!(members(&messages, "GroupId"), ["a", "b", "c"]);
    assert_eq!(members(&messages, "SequenceNumber"), ["1", "2", "4"]);
    // Groups whose head is in flight deliver nothing.
    assert_eq!(client.receive("q.fifo", 10), Vec::<Value>::new());
    let response = client
        .call(json!({"Command": "DeleteMessage", "QueueName": "q.fifo", "ReceiptHandle": "r-1"}));
    assert_eq!(response, json!({"ok": true}));
    assert_eq!(members(&client.receive("q.fifo", 10), "Body"), ["a2"]);
}

#[test]
fn fifo_group_waits_behind_a_visible_head_that_reappears() {
    let mut client = Client::default();
    client.create("q.fifo", json!({"Fifo": true, "VisibilityTimeout": 5}));
    for (body, dedup) in [("first", "1"), ("second", "2")] {
        client.call(json!({"Command": "SendMessage", "QueueName": "q.fifo", "Body": body, "GroupId": "g", "DeduplicationId": dedup}));
    }
    assert_eq!(members(&client.receive("q.fifo", 1), "Body"), ["first"]);
    client.tick(5);
    assert_eq!(members(&client.receive("q.fifo", 10), "Body"), ["first"]);
}

#[test]
fn fifo_deduplication_window_is_300_seconds_from_acceptance() {
    let mut client = Client::default();
    client.create("q.fifo", json!({"Fifo": true}));
    let send = |group: &str| json!({"Command": "SendMessage", "QueueName": "q.fifo", "Body": "x", "GroupId": group, "DeduplicationId": "d"});
    assert_eq!(
        client.call(send("g")),
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
    client.tick(299);
    // Duplicates ignore the group and do not extend the window.
    assert_eq!(
        client.call(send("h")),
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
    client.receive("q.fifo", 1);
    client.call(json!({"Command": "DeleteMessage", "QueueName": "q.fifo", "ReceiptHandle": "r-1"}));
    assert_eq!(
        client.call(send("g")),
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
    client.tick(1);
    assert_eq!(
        client.call(send("g")),
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2})
    );
    assert_eq!(client.counts("q.fifo"), [1, 0, 0]);
}

#[test]
fn content_based_deduplication_uses_the_body() {
    let mut client = Client::default();
    client.create(
        "q.fifo",
        json!({"Fifo": true, "ContentBasedDeduplication": true}),
    );
    let send = |body: &str| json!({"Command": "SendMessage", "QueueName": "q.fifo", "Body": body, "GroupId": "g"});
    assert_eq!(
        client.call(send("same")),
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
    assert_eq!(
        client.call(send("same")),
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
    assert_eq!(
        client.call(send("other")),
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2})
    );
    // A DeduplicationId equal to an earlier body is the same id.
    let response = client.call(json!({"Command": "SendMessage", "QueueName": "q.fifo", "Body": "z", "GroupId": "g", "DeduplicationId": "other"}));
    assert_eq!(
        response,
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2})
    );
}

#[test]
fn deduplication_is_per_queue() {
    let mut client = Client::default();
    client.create("a.fifo", json!({"Fifo": true}));
    client.create("b.fifo", json!({"Fifo": true}));
    for (queue, want) in [("a.fifo", "m-1"), ("b.fifo", "m-2")] {
        let response = client.call(json!({"Command": "SendMessage", "QueueName": queue, "Body": "x", "GroupId": "g", "DeduplicationId": "d"}));
        assert_eq!(
            response,
            json!({"ok": true, "MessageId": want, "SequenceNumber": 1}),
            "queue {queue}"
        );
    }
}

#[test]
fn messages_received_too_often_move_to_the_dead_letter_queue() {
    let mut client = Client::default();
    client.create("dlq", json!({}));
    client.create(
        "jobs",
        json!({"VisibilityTimeout": 0, "DeadLetterQueue": "dlq", "MaxReceiveCount": 2}),
    );
    client.tick(10);
    client.send("jobs", "poison");
    client.send("jobs", "fine");
    assert_eq!(members(&client.receive("jobs", 1), "Body"), ["poison"]);
    assert_eq!(members(&client.receive("jobs", 1), "Body"), ["poison"]);
    client.tick(1);
    // The move does not count toward MaxMessages and the scan goes on.
    let messages = client.receive("jobs", 1);
    assert_eq!(members(&messages, "Body"), ["fine"]);
    assert_eq!(client.counts("jobs"), [1, 0, 0]);
    assert_eq!(client.counts("dlq"), [1, 0, 0]);
    let response = client.call(json!({"Command": "ReceiveMessage", "QueueName": "dlq"}));
    assert_eq!(
        response,
        json!({"ok": true, "Messages": [{
            "MessageId": "m-1",
            "ReceiptHandle": "r-4",
            "Body": "poison",
            "ReceiveCount": 1,
            "SentAt": 10,
            "FirstReceivedAt": 11
        }]})
    );
    // Handles issued in the source queue are gone with the message.
    let code = client
        .error(json!({"Command": "DeleteMessage", "QueueName": "dlq", "ReceiptHandle": "r-2"}));
    assert_eq!(code, "ReceiptHandleExpired");
}

#[test]
fn dead_lettered_fifo_message_takes_the_next_sequence_number() {
    let mut client = Client::default();
    client.create(
        "dlq.fifo",
        json!({"Fifo": true, "ContentBasedDeduplication": true}),
    );
    client.call(json!({"Command": "SendMessage", "QueueName": "dlq.fifo", "Body": "resident", "GroupId": "z"}));
    client.create(
        "q.fifo",
        json!({"Fifo": true, "ContentBasedDeduplication": true, "VisibilityTimeout": 0,
        "DeadLetterQueue": "dlq.fifo", "MaxReceiveCount": 1}),
    );
    for (body, group) in [("a1", "a"), ("a2", "a")] {
        client.call(json!({"Command": "SendMessage", "QueueName": "q.fifo", "Body": body, "GroupId": group}));
    }
    assert_eq!(members(&client.receive("q.fifo", 10), "Body"), ["a1"]);
    // a1 leaves its group, so a2 is the head and delivered in the same scan.
    let messages = client.receive("q.fifo", 10);
    assert_eq!(members(&messages, "Body"), ["a2"]);
    let messages = client.receive("dlq.fifo", 10);
    assert_eq!(members(&messages, "Body"), ["resident", "a1"]);
    assert_eq!(members(&messages, "GroupId"), ["z", "a"]);
    assert_eq!(members(&messages, "SequenceNumber"), ["1", "2"]);
    assert_eq!(members(&messages, "MessageId"), ["m-1", "m-2"]);
}

#[test]
fn dead_lettered_message_past_retention_there_is_removed() {
    let mut client = Client::default();
    client.create("dlq", json!({"RetentionSeconds": 60}));
    client.create(
        "jobs",
        json!({"VisibilityTimeout": 0, "DeadLetterQueue": "dlq", "MaxReceiveCount": 1}),
    );
    client.send("jobs", "old");
    client.receive("jobs", 1);
    client.tick(60);
    assert_eq!(client.receive("jobs", 1), Vec::<Value>::new());
    assert_eq!(client.counts("jobs"), [0, 0, 0]);
    assert_eq!(client.counts("dlq"), [0, 0, 0]);
}

#[test]
fn new_dead_letter_configuration_applies_on_the_next_receive() {
    let mut client = Client::default();
    client.create("dlq", json!({}));
    client.create("jobs", json!({"VisibilityTimeout": 0}));
    client.send("jobs", "a");
    client.receive("jobs", 1);
    client.receive("jobs", 1);
    client.call(json!({"Command": "SetQueueAttributes", "QueueName": "jobs",
        "Attributes": {"DeadLetterQueue": "dlq", "MaxReceiveCount": 2}}));
    assert_eq!(client.receive("jobs", 1), Vec::<Value>::new());
    assert_eq!(client.counts("dlq"), [1, 0, 0]);
}

#[test]
fn a_queue_holds_at_most_100000_messages() {
    let mut client = Client::default();
    client.create("jobs", json!({}));
    client.create("dlq", json!({}));
    for _ in 0..100_000 {
        client.send("jobs", "x");
    }
    let code = client.error(json!({"Command": "SendMessage", "QueueName": "jobs", "Body": "x"}));
    assert_eq!(code, "LimitExceeded");
    assert_eq!(client.counts("jobs"), [100_000, 0, 0]);
    // A failed send uses up no MessageId.
    assert_eq!(client.send("dlq", "y"), "m-100001");
}
