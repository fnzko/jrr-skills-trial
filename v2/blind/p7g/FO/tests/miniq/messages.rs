use crate::common::*;
use serde_json::{Value, json};

fn delete(queue: &str, handle: &str) -> Value {
    json!({"Command": "DeleteMessage", "QueueName": queue, "ReceiptHandle": handle})
}

fn change(queue: &str, handle: &str, timeout: u32) -> Value {
    json!({"Command": "ChangeMessageVisibility", "QueueName": queue, "ReceiptHandle": handle, "VisibilityTimeout": timeout})
}

fn fifo(name: &str) -> Value {
    create_with(name, json!({"Fifo": true}))
}

fn send_fifo(queue: &str, body: &str, group: &str, dedup: &str) -> Value {
    json!({"Command": "SendMessage", "QueueName": queue, "Body": body, "GroupId": group, "DeduplicationId": dedup})
}

#[test]
fn a_received_message_is_in_flight_until_its_timeout_ends() {
    let responses = session(&[
        create_with("q", json!({"VisibilityTimeout": 10})),
        send("q", "hello"),
        tick(3),
        receive("q"),
        receive("q"),
        tick(9),
        attributes("q"),
        tick(1),
        receive("q"),
    ]);
    assert_eq!(responses[1], json!({"ok": true, "MessageId": "m-1"}));
    assert_eq!(
        responses[3],
        json!({"ok": true, "Messages": [{
            "MessageId": "m-1", "ReceiptHandle": "r-1", "Body": "hello",
            "ReceiveCount": 1, "SentAt": 0, "FirstReceivedAt": 3
        }]})
    );
    assert_eq!(responses[4], json!({"ok": true, "Messages": []}));
    assert_eq!(
        responses[6]["Counts"],
        json!({"Visible": 0, "InFlight": 1, "Delayed": 0})
    );
    assert_eq!(
        responses[8],
        json!({"ok": true, "Messages": [{
            "MessageId": "m-1", "ReceiptHandle": "r-2", "Body": "hello",
            "ReceiveCount": 2, "SentAt": 0, "FirstReceivedAt": 3
        }]})
    );
}

#[test]
fn send_and_receive_validate_their_parameters() {
    let responses = session(&[
        create("q"),
        json!({"Command": "SendMessage", "QueueName": "q"}),
        send("q", ""),
        send("q", &"x".repeat(1025)),
        send("q", &"é".repeat(512)),
        json!({"Command": "SendMessage", "QueueName": "q", "Body": "b", "DelaySeconds": 901}),
        json!({"Command": "SendMessage", "QueueName": "q", "Body": "b", "GroupId": "g"}),
        json!({"Command": "SendMessage", "QueueName": "q", "Body": "b", "DeduplicationId": "bad id"}),
        send("nope", "b"),
        json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 0}),
        json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 11}),
        json!({"Command": "ReceiveMessage", "QueueName": "q", "VisibilityTimeout": 43201}),
        receive("nope"),
    ]);
    assert_error(&responses[1], "MissingParameter");
    assert_error(&responses[2], "InvalidParameterValue");
    assert_error(&responses[3], "InvalidParameterValue");
    assert_eq!(responses[4], json!({"ok": true, "MessageId": "m-1"}));
    assert_error(&responses[5], "InvalidParameterValue");
    assert_error(&responses[6], "UnsupportedOperation");
    assert_error(&responses[7], "InvalidParameterValue");
    assert_error(&responses[8], "QueueDoesNotExist");
    assert_error(&responses[9], "InvalidParameterValue");
    assert_error(&responses[10], "InvalidParameterValue");
    assert_error(&responses[11], "InvalidParameterValue");
    assert_error(&responses[12], "QueueDoesNotExist");
}

#[test]
fn delays_hide_messages_and_keep_arrival_order() {
    let responses = session(&[
        create_with("q", json!({"DelaySeconds": 10})),
        send("q", "queue-default"),
        json!({"Command": "SendMessage", "QueueName": "q", "Body": "now", "DelaySeconds": 0}),
        json!({"Command": "SendMessage", "QueueName": "q", "Body": "late", "DelaySeconds": 20}),
        attributes("q"),
        tick(10),
        receive_max("q", 10),
        tick(10),
        receive_max("q", 10),
    ]);
    assert_eq!(
        responses[4]["Counts"],
        json!({"Visible": 1, "InFlight": 0, "Delayed": 2})
    );
    assert_eq!(bodies(&responses[6]), ["queue-default", "now"]);
    assert_eq!(bodies(&responses[8]), ["late"]);
}

#[test]
fn receive_delivers_up_to_max_in_arrival_order_with_a_per_call_timeout() {
    let responses = session(&[
        create("q"),
        send("q", "a"),
        send("q", "b"),
        send("q", "c"),
        json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 2, "VisibilityTimeout": 0}),
        receive_max("q", 10),
    ]);
    assert_eq!(bodies(&responses[4]), ["a", "b"]);
    assert_eq!(bodies(&responses[5]), ["a", "b", "c"]);
    let handles: Vec<_> = responses[4]["Messages"]
        .as_array()
        .into_iter()
        .chain(responses[5]["Messages"].as_array())
        .flatten()
        .map(|m| m["ReceiptHandle"].clone())
        .collect();
    assert_eq!(handles, ["r-1", "r-2", "r-3", "r-4", "r-5"]);
}

#[test]
fn receipt_handles_are_checked_in_the_written_order() {
    let responses = session(&[
        create("q"),
        create("other"),
        send("q", "a"),
        receive("q"),
        delete("q", "r-01"),
        delete("q", "R-1"),
        delete("q", "r-1 "),
        delete("q", "r-2"),
        delete("other", "r-1"),
        delete("nope", "bogus"),
        change("q", "r-1", 43201),
        change("q", "r-1", 0),
        delete("q", "r-1"),
        receive("q"),
        delete("q", "r-1"),
        delete("q", "r-2"),
        delete("q", "r-2"),
        json!({"Command": "DeleteMessage", "QueueName": "q"}),
    ]);
    assert_error(&responses[4], "ReceiptHandleInvalid");
    assert_error(&responses[5], "ReceiptHandleInvalid");
    assert_error(&responses[6], "ReceiptHandleInvalid");
    assert_error(&responses[7], "ReceiptHandleInvalid");
    assert_error(&responses[8], "ReceiptHandleExpired");
    assert_error(&responses[9], "QueueDoesNotExist");
    assert_error(&responses[10], "InvalidParameterValue");
    assert_eq!(responses[11], ok());
    assert_error(&responses[12], "ReceiptHandleExpired");
    assert_error(&responses[14], "ReceiptHandleExpired");
    assert_eq!(responses[15], ok());
    assert_error(&responses[16], "ReceiptHandleExpired");
    assert_error(&responses[17], "MissingParameter");
}

#[test]
fn change_visibility_replaces_the_end_of_the_timeout() {
    let responses = session(&[
        create_with("q", json!({"VisibilityTimeout": 10})),
        send("q", "a"),
        receive("q"),
        tick(5),
        change("q", "r-1", 20),
        tick(19),
        receive("q"),
        tick(1),
        receive("q"),
        tick(30),
        change("q", "r-2", 5),
    ]);
    assert_eq!(responses[4], ok());
    assert_eq!(responses[6], json!({"ok": true, "Messages": []}));
    assert_eq!(responses[8]["Messages"][0]["ReceiveCount"], json!(2));
    assert_error(&responses[10], "ReceiptHandleExpired");
}

#[test]
fn a_zero_timeout_makes_the_handle_useless_at_once() {
    let responses = session(&[
        create("q"),
        send("q", "a"),
        json!({"Command": "ReceiveMessage", "QueueName": "q", "VisibilityTimeout": 0}),
        delete("q", "r-1"),
        attributes("q"),
    ]);
    assert_error(&responses[3], "ReceiptHandleExpired");
    assert_eq!(
        responses[4]["Counts"],
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn retention_removes_messages_in_every_state_and_is_not_undone() {
    let responses = session(&[
        create_with(
            "q",
            json!({"RetentionSeconds": 100, "VisibilityTimeout": 1000}),
        ),
        send("q", "visible"),
        send("q", "taken"),
        receive("q"),
        json!({"Command": "SendMessage", "QueueName": "q", "Body": "delayed", "DelaySeconds": 900}),
        tick(99),
        attributes("q"),
        tick(1),
        attributes("q"),
        send("q", "fresh"),
        tick(70),
        json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"RetentionSeconds": 60}}),
        json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"RetentionSeconds": 1000}}),
        attributes("q"),
    ]);
    assert_eq!(
        responses[6]["Counts"],
        json!({"Visible": 1, "InFlight": 1, "Delayed": 1})
    );
    assert_eq!(
        responses[8]["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(
        responses[13]["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn purge_removes_everything_but_keeps_sequence_numbers_and_deduplication() {
    let responses = session(&[
        fifo("f.fifo"),
        send_fifo("f.fifo", "a", "g", "d1"),
        send_fifo("f.fifo", "b", "g", "d2"),
        json!({"Command": "PurgeQueue", "QueueName": "f.fifo"}),
        send_fifo("f.fifo", "again", "g", "d1"),
        send_fifo("f.fifo", "c", "g", "d3"),
        json!({"Command": "PurgeQueue", "QueueName": "nope"}),
    ]);
    assert_eq!(responses[3], ok());
    assert_eq!(
        responses[4],
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
    assert_eq!(
        responses[5],
        json!({"ok": true, "MessageId": "m-3", "SequenceNumber": 3})
    );
    assert_error(&responses[6], "QueueDoesNotExist");
}

#[test]
fn fifo_send_rules_apply_in_the_written_order() {
    let responses = session(&[
        fifo("f.fifo"),
        create("s"),
        json!({"Command": "SendMessage", "QueueName": "s", "Body": "b", "GroupId": "g"}),
        json!({"Command": "SendMessage", "QueueName": "s", "Body": "b", "DeduplicationId": "d"}),
        json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "b", "DelaySeconds": 1}),
        json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "b", "DeduplicationId": "d"}),
        json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "b", "GroupId": "g"}),
        json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "b", "GroupId": "g", "DeduplicationId": "d"}),
    ]);
    assert_error(&responses[2], "UnsupportedOperation");
    assert_error(&responses[3], "UnsupportedOperation");
    assert_error(&responses[4], "UnsupportedOperation");
    assert_error(&responses[5], "MissingParameter");
    assert_error(&responses[6], "MissingParameter");
    assert_eq!(
        responses[7],
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
}

#[test]
fn duplicates_return_the_original_until_the_window_closes() {
    let responses = session(&[
        fifo("f.fifo"),
        send_fifo("f.fifo", "a", "g1", "d"),
        tick(299),
        send_fifo("f.fifo", "b", "g2", "d"),
        tick(1),
        send_fifo("f.fifo", "c", "g3", "d"),
        send_fifo("f.fifo", "d", "g3", "d"),
        receive_max("f.fifo", 10),
    ]);
    assert_eq!(
        responses[1],
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
    assert_eq!(responses[3], responses[1]);
    assert_eq!(
        responses[5],
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2})
    );
    assert_eq!(responses[6], responses[5]);
    assert_eq!(bodies(&responses[7]), ["a", "c"]);
}

#[test]
fn content_based_deduplication_uses_the_body() {
    let responses = session(&[
        create_with(
            "f.fifo",
            json!({"Fifo": true, "ContentBasedDeduplication": true}),
        ),
        json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "x", "GroupId": "g"}),
        json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "x", "GroupId": "h"}),
        json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "y", "GroupId": "g"}),
        send_fifo("f.fifo", "x", "g", "x"),
    ]);
    assert_eq!(responses[2], responses[1]);
    assert_eq!(
        responses[3],
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2})
    );
    assert_eq!(responses[4], responses[1]);
}

#[test]
fn a_dedup_id_outlives_its_message() {
    let responses = session(&[
        fifo("f.fifo"),
        send_fifo("f.fifo", "a", "g", "d"),
        receive("f.fifo"),
        delete("f.fifo", "r-1"),
        send_fifo("f.fifo", "a", "g", "d"),
        attributes("f.fifo"),
    ]);
    assert_eq!(responses[4], responses[1]);
    assert_eq!(
        responses[5]["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn fifo_groups_deliver_one_message_at_a_time_and_in_order() {
    let responses = session(&[
        fifo("f.fifo"),
        send_fifo("f.fifo", "a1", "a", "1"),
        send_fifo("f.fifo", "b1", "b", "2"),
        send_fifo("f.fifo", "a2", "a", "3"),
        send_fifo("f.fifo", "b2", "b", "4"),
        receive_max("f.fifo", 10),
        receive_max("f.fifo", 10),
        delete("f.fifo", "r-1"),
        receive_max("f.fifo", 10),
    ]);
    assert_eq!(bodies(&responses[5]), ["a1", "b1"]);
    assert_eq!(responses[5]["Messages"][1]["GroupId"], json!("b"));
    assert_eq!(responses[5]["Messages"][1]["SequenceNumber"], json!(2));
    assert_eq!(bodies(&responses[6]), Vec::<&str>::new());
    assert_eq!(bodies(&responses[8]), ["a2"]);
}

#[test]
fn messages_beyond_the_dead_letter_threshold_move_on_the_next_receive() {
    let responses = session(&[
        create_with("dlq", json!({"RetentionSeconds": 100})),
        create_with(
            "src",
            json!({"DeadLetterQueue": "dlq", "MaxReceiveCount": 2}),
        ),
        send("src", "poison"),
        send("src", "fine"),
        json!({"Command": "ReceiveMessage", "QueueName": "src", "VisibilityTimeout": 0}),
        json!({"Command": "ReceiveMessage", "QueueName": "src", "VisibilityTimeout": 0, "MaxMessages": 1}),
        json!({"Command": "ReceiveMessage", "QueueName": "src", "VisibilityTimeout": 0, "MaxMessages": 2}),
        attributes("src"),
        receive("dlq"),
        tick(150),
        send("src", "x"),
        receive("src"),
        attributes("dlq"),
    ]);
    assert_eq!(bodies(&responses[4]), ["poison"]);
    assert_eq!(bodies(&responses[5]), ["poison"]);
    // Both messages are examined: "poison" has been received twice and moves, "fine" is delivered.
    assert_eq!(bodies(&responses[6]), ["fine"]);
    assert_eq!(
        responses[7]["Counts"],
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(
        responses[8],
        json!({"ok": true, "Messages": [{
            "MessageId": "m-1", "ReceiptHandle": "r-4", "Body": "poison",
            "ReceiveCount": 1, "SentAt": 0, "FirstReceivedAt": 0
        }]})
    );
}

#[test]
fn a_dead_lettered_message_that_is_too_old_for_the_target_is_dropped() {
    let responses = session(&[
        create_with("dlq", json!({"RetentionSeconds": 60})),
        create_with(
            "src",
            json!({"DeadLetterQueue": "dlq", "MaxReceiveCount": 1}),
        ),
        send("src", "old"),
        json!({"Command": "ReceiveMessage", "QueueName": "src", "VisibilityTimeout": 70}),
        tick(70),
        receive("src"),
        attributes("dlq"),
    ]);
    assert_eq!(responses[5], json!({"ok": true, "Messages": []}));
    assert_eq!(
        responses[6]["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn dead_lettering_a_fifo_message_takes_the_next_sequence_number_of_the_target() {
    let responses = session(&[
        fifo("dlq.fifo"),
        create_with(
            "src.fifo",
            json!({"Fifo": true, "DeadLetterQueue": "dlq.fifo", "MaxReceiveCount": 1}),
        ),
        send_fifo("dlq.fifo", "native", "g", "n"),
        send_fifo("src.fifo", "moved", "h", "m"),
        send_fifo("src.fifo", "next", "h", "m2"),
        json!({"Command": "ReceiveMessage", "QueueName": "src.fifo", "VisibilityTimeout": 0}),
        receive("src.fifo"),
        receive_max("dlq.fifo", 10),
    ]);
    // The moved message no longer blocks its group, so "next" is delivered in the same call.
    assert_eq!(bodies(&responses[6]), ["next"]);
    assert_eq!(bodies(&responses[7]), ["native", "moved"]);
    assert_eq!(responses[7]["Messages"][1]["SequenceNumber"], json!(2));
    assert_eq!(responses[7]["Messages"][1]["GroupId"], json!("h"));
}

#[test]
fn a_queue_holds_at_most_100000_messages() {
    let mut commands = vec![create("q")];
    commands.extend((0..100_001).map(|_| send("q", "x")));
    commands.push(attributes("q"));
    let responses = session(&commands);
    assert_eq!(
        responses[100_000],
        json!({"ok": true, "MessageId": "m-100000"})
    );
    assert_error(&responses[100_001], "LimitExceeded");
    assert_eq!(responses[100_002]["Counts"]["Visible"], json!(100_000));
}
