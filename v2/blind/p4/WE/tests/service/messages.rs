use serde_json::{Value, json};

use crate::common::{
    check, counts, create, create_with, delete_message, fail, get_attributes, messages, ok,
    receive, responses, send, send_fifo, sent, sent_fifo, tick,
};

fn now(seconds: u32) -> Value {
    json!({"ok": true, "Now": seconds})
}

fn message(id: u32, handle: u32, body: &str, count: u32, sent_at: u32, first: u32) -> Value {
    json!({
        "MessageId": format!("m-{id}"), "ReceiptHandle": format!("r-{handle}"), "Body": body,
        "ReceiveCount": count, "SentAt": sent_at, "FirstReceivedAt": first,
    })
}

fn fifo_message(id: u32, handle: u32, body: &str, count: u32, group: &str, sequence: u32) -> Value {
    let mut message = message(id, handle, body, count, 0, 0);
    message["GroupId"] = json!(group);
    message["SequenceNumber"] = json!(sequence);
    message
}

fn change_visibility(queue: &str, handle: &str, timeout: u32) -> Value {
    json!({"Command": "ChangeMessageVisibility", "QueueName": queue, "ReceiptHandle": handle, "VisibilityTimeout": timeout})
}

fn fifo_queue(name: &str) -> Value {
    create_with(name, json!({"Fifo": true}))
}

fn attributes(queue_counts: Value) -> Value {
    json!({"ok": true, "Attributes": {
        "VisibilityTimeout": 30, "DelaySeconds": 0, "RetentionSeconds": 345_600,
        "Fifo": false, "ContentBasedDeduplication": false,
    }, "Counts": queue_counts})
}

#[test]
fn send_parameter_rules() {
    check(&[
        (create("q"), ok()),
        (fifo_queue("f.fifo"), ok()),
        (
            json!({"Command": "SendMessage", "QueueName": "q", "Body": "x".repeat(1025)}),
            fail("InvalidParameterValue"),
        ),
        (
            json!({"Command": "SendMessage", "QueueName": "q", "Body": "b", "DelaySeconds": 901}),
            fail("InvalidParameterValue"),
        ),
        (
            json!({"Command": "SendMessage", "QueueName": "q", "Body": "b", "GroupId": "g"}),
            fail("UnsupportedOperation"),
        ),
        (
            json!({"Command": "SendMessage", "QueueName": "q", "Body": "b", "DeduplicationId": "d"}),
            fail("UnsupportedOperation"),
        ),
        (
            json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "b", "DelaySeconds": 0, "GroupId": "g", "DeduplicationId": "d"}),
            fail("UnsupportedOperation"),
        ),
        (
            json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "b", "DeduplicationId": "d"}),
            fail("MissingParameter"),
        ),
        (
            json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "b", "GroupId": "g"}),
            fail("MissingParameter"),
        ),
        // Failed sends use up no message id.
        (send("q", "b"), sent(1)),
        (send_fifo("f.fifo", "b", "g", "d"), sent_fifo(2, 1)),
    ]);
}

#[test]
fn delay_hides_messages_until_it_ends() {
    check(&[
        (create_with("q", json!({"DelaySeconds": 5})), ok()),
        (send("q", "queue delay"), sent(1)),
        (
            json!({"Command": "SendMessage", "QueueName": "q", "Body": "no delay", "DelaySeconds": 0}),
            sent(2),
        ),
        (
            get_attributes("q"),
            json!({"ok": true, "Attributes": {
            "VisibilityTimeout": 30, "DelaySeconds": 5, "RetentionSeconds": 345_600,
            "Fifo": false, "ContentBasedDeduplication": false,
        }, "Counts": counts(1, 0, 1)}),
        ),
        (
            receive("q", 10),
            messages(json!([message(2, 1, "no delay", 1, 0, 0)])),
        ),
        (tick(5), now(5)),
        (
            receive("q", 10),
            messages(json!([message(1, 2, "queue delay", 1, 0, 5)])),
        ),
    ]);
}

#[test]
fn visibility_timeout_returns_messages_to_their_place() {
    check(&[
        (create("q"), ok()),
        (send("q", "a"), sent(1)),
        (send("q", "b"), sent(2)),
        (
            json!({"Command": "ReceiveMessage", "QueueName": "q", "VisibilityTimeout": 10}),
            messages(json!([message(1, 1, "a", 1, 0, 0)])),
        ),
        (
            receive("q", 10),
            messages(json!([message(2, 2, "b", 1, 0, 0)])),
        ),
        (tick(10), now(10)),
        (
            receive("q", 10),
            messages(json!([message(1, 3, "a", 2, 0, 0)])),
        ),
        (tick(20), now(30)),
        (
            receive("q", 10),
            messages(json!([message(2, 4, "b", 2, 0, 0)])),
        ),
        (get_attributes("q"), attributes(counts(0, 2, 0))),
        (
            json!({"Command": "ReceiveMessage", "QueueName": "q", "VisibilityTimeout": 43_201}),
            fail("InvalidParameterValue"),
        ),
    ]);
}

#[test]
fn zero_visibility_timeout_never_yields_a_usable_handle() {
    check(&[
        (create_with("q", json!({"VisibilityTimeout": 0})), ok()),
        (send("q", "a"), sent(1)),
        (
            receive("q", 10),
            messages(json!([message(1, 1, "a", 1, 0, 0)])),
        ),
        (delete_message("q", "r-1"), fail("ReceiptHandleExpired")),
        (
            receive("q", 10),
            messages(json!([message(1, 2, "a", 2, 0, 0)])),
        ),
        (
            get_attributes("q"),
            json!({"ok": true, "Attributes": {
            "VisibilityTimeout": 0, "DelaySeconds": 0, "RetentionSeconds": 345_600,
            "Fifo": false, "ContentBasedDeduplication": false,
        }, "Counts": counts(1, 0, 0)}),
        ),
    ]);
}

#[test]
fn receipt_handle_rules() {
    check(&[
        (create("q"), ok()),
        (create("other"), ok()),
        (send("q", "a"), sent(1)),
        (send("other", "b"), sent(2)),
        (
            receive("q", 1),
            messages(json!([message(1, 1, "a", 1, 0, 0)])),
        ),
        (
            receive("other", 1),
            messages(json!([message(2, 2, "b", 1, 0, 0)])),
        ),
        (delete_message("q", "r-3"), fail("ReceiptHandleInvalid")),
        (delete_message("q", "r-01"), fail("ReceiptHandleInvalid")),
        (delete_message("q", "R-1"), fail("ReceiptHandleInvalid")),
        (delete_message("q", "r-1 "), fail("ReceiptHandleInvalid")),
        (
            change_visibility("q", "r-0", 5),
            fail("ReceiptHandleInvalid"),
        ),
        (delete_message("q", "r-2"), fail("ReceiptHandleExpired")),
        (
            change_visibility("q", "r-1", 43_201),
            fail("InvalidParameterValue"),
        ),
        (
            json!({"Command": "ChangeMessageVisibility", "QueueName": "q", "ReceiptHandle": "r-1"}),
            fail("MissingParameter"),
        ),
        (change_visibility("q", "r-1", 100), ok()),
        (change_visibility("other", "r-2", 0), ok()),
        (
            change_visibility("other", "r-2", 10),
            fail("ReceiptHandleExpired"),
        ),
        (tick(99), now(99)),
        (delete_message("q", "r-1"), ok()),
        (delete_message("q", "r-1"), fail("ReceiptHandleExpired")),
        (
            receive("other", 1),
            messages(json!([message(2, 3, "b", 2, 0, 0)])),
        ),
        (delete_message("other", "r-2"), fail("ReceiptHandleExpired")),
        (delete_message("other", "r-3"), ok()),
        (get_attributes("q"), attributes(counts(0, 0, 0))),
    ]);
}

#[test]
fn fifo_receive_delivers_one_message_per_free_group() {
    check(&[
        (fifo_queue("f.fifo"), ok()),
        (send_fifo("f.fifo", "a1", "a", "1"), sent_fifo(1, 1)),
        (send_fifo("f.fifo", "a2", "a", "2"), sent_fifo(2, 2)),
        (send_fifo("f.fifo", "b1", "b", "3"), sent_fifo(3, 3)),
        (
            receive("f.fifo", 10),
            messages(json!([
                fifo_message(1, 1, "a1", 1, "a", 1),
                fifo_message(3, 2, "b1", 1, "b", 3),
            ])),
        ),
        (receive("f.fifo", 10), messages(json!([]))),
        (delete_message("f.fifo", "r-1"), ok()),
        (
            receive("f.fifo", 10),
            messages(json!([fifo_message(2, 3, "a2", 1, "a", 2)])),
        ),
    ]);
}

#[test]
fn fifo_deduplication_window() {
    check(&[
        (fifo_queue("f.fifo"), ok()),
        (send_fifo("f.fifo", "x", "g", "d"), sent_fifo(1, 1)),
        // Same id in another group and with another body is still a duplicate.
        (tick(299), now(299)),
        (send_fifo("f.fifo", "y", "h", "d"), sent_fifo(1, 1)),
        (
            receive("f.fifo", 10),
            messages(json!([{
                "MessageId": "m-1", "ReceiptHandle": "r-1", "Body": "x", "ReceiveCount": 1,
                "SentAt": 0, "FirstReceivedAt": 299, "GroupId": "g", "SequenceNumber": 1,
            }])),
        ),
        // Deleting the original does not free the id.
        (delete_message("f.fifo", "r-1"), ok()),
        (
            json!({"Command": "PurgeQueue", "QueueName": "f.fifo"}),
            ok(),
        ),
        (send_fifo("f.fifo", "y", "h", "d"), sent_fifo(1, 1)),
        // The duplicate did not extend the window.
        (tick(1), now(300)),
        (send_fifo("f.fifo", "z", "g", "d"), sent_fifo(2, 2)),
        (send_fifo("f.fifo", "z", "g", "e"), sent_fifo(3, 3)),
    ]);
}

#[test]
fn content_based_deduplication_uses_the_body() {
    check(&[
        (
            create_with(
                "f.fifo",
                json!({"Fifo": true, "ContentBasedDeduplication": true}),
            ),
            ok(),
        ),
        (
            json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "same", "GroupId": "g"}),
            sent_fifo(1, 1),
        ),
        (
            json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "same", "GroupId": "h"}),
            sent_fifo(1, 1),
        ),
        // An explicit id takes precedence, and shares the id space with bodies.
        (send_fifo("f.fifo", "same", "g", "other"), sent_fifo(2, 2)),
        (
            json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "other", "GroupId": "g"}),
            sent_fifo(2, 2),
        ),
    ]);
}

#[test]
fn dead_letter_move_after_max_receive_count() {
    check(&[
        (create_with("dlq.fifo", json!({"Fifo": true})), ok()),
        (
            create_with(
                "f.fifo",
                json!({"Fifo": true, "DeadLetterQueue": "dlq.fifo", "MaxReceiveCount": 1, "VisibilityTimeout": 5}),
            ),
            ok(),
        ),
        (send_fifo("dlq.fifo", "resident", "z", "r"), sent_fifo(1, 1)),
        (send_fifo("f.fifo", "a1", "a", "1"), sent_fifo(2, 1)),
        (send_fifo("f.fifo", "a2", "a", "2"), sent_fifo(3, 2)),
        (
            receive("f.fifo", 1),
            messages(json!([fifo_message(2, 1, "a1", 1, "a", 1)])),
        ),
        (tick(5), now(5)),
        // a1 moves, which frees group a for a2 within the same receive.
        (
            receive("f.fifo", 1),
            messages(json!([{
                "MessageId": "m-3", "ReceiptHandle": "r-2", "Body": "a2", "ReceiveCount": 1,
                "SentAt": 0, "FirstReceivedAt": 5, "GroupId": "a", "SequenceNumber": 2,
            }])),
        ),
        (
            delete_message("f.fifo", "r-1"),
            fail("ReceiptHandleExpired"),
        ),
        (
            receive("dlq.fifo", 10),
            messages(json!([
                {
                    "MessageId": "m-1", "ReceiptHandle": "r-3", "Body": "resident", "ReceiveCount": 1,
                    "SentAt": 0, "FirstReceivedAt": 5, "GroupId": "z", "SequenceNumber": 1,
                },
                {
                    "MessageId": "m-2", "ReceiptHandle": "r-4", "Body": "a1", "ReceiveCount": 1,
                    "SentAt": 0, "FirstReceivedAt": 5, "GroupId": "a", "SequenceNumber": 2,
                },
            ])),
        ),
    ]);
}

#[test]
fn dead_letter_move_drops_messages_past_the_target_retention() {
    check(&[
        (create_with("dlq", json!({"RetentionSeconds": 60})), ok()),
        (
            create_with(
                "q",
                json!({"DeadLetterQueue": "dlq", "MaxReceiveCount": 1, "VisibilityTimeout": 0}),
            ),
            ok(),
        ),
        (send("q", "m"), sent(1)),
        (
            receive("q", 1),
            messages(json!([message(1, 1, "m", 1, 0, 0)])),
        ),
        (tick(60), now(60)),
        (receive("q", 1), messages(json!([]))),
        (
            get_attributes("q"),
            json!({"ok": true, "Attributes": {
            "VisibilityTimeout": 0, "DelaySeconds": 0, "RetentionSeconds": 345_600,
            "Fifo": false, "ContentBasedDeduplication": false,
            "DeadLetterQueue": "dlq", "MaxReceiveCount": 1,
        }, "Counts": counts(0, 0, 0)}),
        ),
        (
            get_attributes("dlq"),
            json!({"ok": true, "Attributes": {
            "VisibilityTimeout": 30, "DelaySeconds": 0, "RetentionSeconds": 60,
            "Fifo": false, "ContentBasedDeduplication": false,
        }, "Counts": counts(0, 0, 0)}),
        ),
    ]);
}

#[test]
fn queue_holds_at_most_one_hundred_thousand_messages() {
    let mut input = format!("{}\n", create("q"));
    for _ in 0..100_000 {
        input.push_str(r#"{"Command":"SendMessage","QueueName":"q","Body":"b"}"#);
        input.push('\n');
    }
    input.push_str(&format!("{}\n", send("q", "one too many")));
    input.push_str(&format!("{}\n", get_attributes("q")));
    let responses = responses(input.as_bytes());
    assert_eq!(responses.len(), 100_003);
    assert_eq!(responses[100_000], sent(100_000));
    assert_eq!(responses[100_001], fail("LimitExceeded"));
    assert_eq!(responses[100_002], attributes(counts(100_000, 0, 0)));
}
