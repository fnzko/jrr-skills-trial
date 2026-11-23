mod common;

use common::{code, exchange, ok};
use serde_json::{Value, json};

fn create(name: &str, attributes: Value) -> Value {
    json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attributes})
}

fn send(queue: &str, body: &str) -> Value {
    json!({"Command": "SendMessage", "QueueName": queue, "Body": body})
}

fn receive(queue: &str) -> Value {
    json!({"Command": "ReceiveMessage", "QueueName": queue})
}

fn tick(seconds: u32) -> Value {
    json!({"Command": "Tick", "Seconds": seconds})
}

fn counts(queue: &str) -> Value {
    json!({"Command": "GetQueueAttributes", "QueueName": queue})
}

fn message(
    id: &str,
    handle: &str,
    body: &str,
    receive_count: u32,
    sent_at: u32,
    first: u32,
) -> Value {
    json!({
        "MessageId": id, "ReceiptHandle": handle, "Body": body,
        "ReceiveCount": receive_count, "SentAt": sent_at, "FirstReceivedAt": first,
    })
}

#[test]
fn a_message_reappears_after_its_visibility_timeout_in_arrival_order() {
    let responses = exchange(&[
        create("q", json!({"VisibilityTimeout": 10})),
        send("q", "first"),
        send("q", "second"),
        receive("q"),
        tick(9),
        receive("q"),
        tick(10),
        json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 10}),
    ]);
    assert_eq!(
        responses[3],
        json!({"ok": true, "Messages": [message("m-1", "r-1", "first", 1, 0, 0)]})
    );
    assert_eq!(
        responses[5],
        json!({"ok": true, "Messages": [message("m-2", "r-2", "second", 1, 0, 9)]})
    );
    assert_eq!(
        responses[7],
        json!({"ok": true, "Messages": [
            message("m-1", "r-3", "first", 2, 0, 0),
            message("m-2", "r-4", "second", 2, 0, 9),
        ]})
    );
}

#[test]
fn only_the_latest_receipt_handle_of_an_in_flight_message_is_usable() {
    let delete = |handle: &str| json!({"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": handle});
    let responses = exchange(&[
        create("q", json!({"VisibilityTimeout": 10})),
        create("other", json!({})),
        send("q", "a"),
        receive("q"),
        tick(10),
        receive("q"),
        delete("r-1"),
        delete("r-01"),
        delete("R-2"),
        delete("r-2 "),
        delete("r-3"),
        json!({"Command": "DeleteMessage", "QueueName": "other", "ReceiptHandle": "r-2"}),
        delete("r-2"),
        delete("r-2"),
        counts("q"),
    ]);
    let codes: Vec<_> = responses[6..14].iter().map(|r| code(r).clone()).collect();
    assert_eq!(
        codes,
        [
            json!("ReceiptHandleExpired"),
            json!("ReceiptHandleInvalid"),
            json!("ReceiptHandleInvalid"),
            json!("ReceiptHandleInvalid"),
            json!("ReceiptHandleInvalid"),
            json!("ReceiptHandleExpired"),
            json!(null),
            json!("ReceiptHandleExpired"),
        ]
    );
}

#[test]
fn a_visibility_timeout_of_zero_makes_the_message_visible_at_once() {
    let responses = exchange(&[
        create("q", json!({"VisibilityTimeout": 0})),
        send("q", "a"),
        receive("q"),
        json!({"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": "r-1"}),
        json!({"Command": "ReceiveMessage", "QueueName": "q", "VisibilityTimeout": 5}),
        json!({"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": "r-2"}),
    ]);
    assert_eq!(code(&responses[3]), "ReceiptHandleExpired");
    assert_eq!(
        responses[4],
        json!({"ok": true, "Messages": [message("m-1", "r-2", "a", 2, 0, 0)]})
    );
    assert_eq!(responses[5], ok());
}

#[test]
fn change_message_visibility_replaces_the_end_of_the_timeout() {
    let change = |handle: &str, timeout: u32| {
        json!({"Command": "ChangeMessageVisibility", "QueueName": "q",
               "ReceiptHandle": handle, "VisibilityTimeout": timeout})
    };
    let responses = exchange(&[
        create("q", json!({"VisibilityTimeout": 10})),
        send("q", "a"),
        receive("q"),
        tick(4),
        change("r-1", 100),
        tick(50),
        receive("q"),
        change("r-1", 0),
        counts("q"),
        change("r-1", 0),
        change("r-9", 5),
        json!({"Command": "ChangeMessageVisibility", "QueueName": "q",
               "ReceiptHandle": "r-1", "VisibilityTimeout": 43201}),
        tick(46),
        receive("q"),
    ]);
    assert_eq!(responses[4], ok());
    assert_eq!(responses[6], json!({"ok": true, "Messages": []}));
    assert_eq!(responses[7], ok());
    assert_eq!(
        responses[8]["Counts"],
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(code(&responses[9]), "ReceiptHandleExpired");
    assert_eq!(code(&responses[10]), "ReceiptHandleInvalid");
    assert_eq!(code(&responses[11]), "InvalidParameterValue");
    assert_eq!(
        responses[13],
        json!({"ok": true, "Messages": [message("m-1", "r-2", "a", 2, 0, 0)]})
    );
}

#[test]
fn delayed_messages_become_visible_when_their_delay_ends() {
    let responses = exchange(&[
        create("q", json!({"DelaySeconds": 5})),
        send("q", "queue delay"),
        json!({"Command": "SendMessage", "QueueName": "q", "Body": "no delay", "DelaySeconds": 0}),
        json!({"Command": "SendMessage", "QueueName": "q", "Body": "long", "DelaySeconds": 900}),
        json!({"Command": "SendMessage", "QueueName": "q", "Body": "bad", "DelaySeconds": 901}),
        counts("q"),
        tick(5),
        counts("q"),
        json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 10}),
    ]);
    assert_eq!(code(&responses[4]), "InvalidParameterValue");
    assert_eq!(
        responses[5]["Counts"],
        json!({"Visible": 1, "InFlight": 0, "Delayed": 2})
    );
    assert_eq!(
        responses[7]["Counts"],
        json!({"Visible": 2, "InFlight": 0, "Delayed": 1})
    );
    assert_eq!(
        responses[8],
        json!({"ok": true, "Messages": [
            message("m-1", "r-1", "queue delay", 1, 0, 5),
            message("m-2", "r-2", "no delay", 1, 0, 5),
        ]})
    );
}

#[test]
fn a_new_queue_delay_applies_to_later_sends_only() {
    let responses = exchange(&[
        create("q", json!({})),
        send("q", "before"),
        json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"DelaySeconds": 9}}),
        send("q", "after"),
        counts("q"),
    ]);
    assert_eq!(
        responses[4]["Counts"],
        json!({"Visible": 1, "InFlight": 0, "Delayed": 1})
    );
}

#[test]
fn messages_are_removed_when_their_retention_runs_out() {
    let responses = exchange(&[
        create("short", json!({"RetentionSeconds": 60})),
        create("long", json!({})),
        send("short", "a"),
        send("long", "b"),
        receive("short"),
        tick(59),
        counts("short"),
        tick(1),
        counts("short"),
        json!({"Command": "DeleteMessage", "QueueName": "short", "ReceiptHandle": "r-1"}),
        json!({"Command": "SetQueueAttributes", "QueueName": "long", "Attributes": {"RetentionSeconds": 60}}),
        counts("long"),
        json!({"Command": "SetQueueAttributes", "QueueName": "long", "Attributes": {"RetentionSeconds": 1000}}),
        counts("long"),
    ]);
    assert_eq!(
        responses[6]["Counts"],
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(
        responses[8]["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(code(&responses[9]), "ReceiptHandleExpired");
    assert_eq!(
        responses[11]["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(
        responses[13]["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn send_message_checks_the_body_and_what_the_queue_type_supports() {
    let long_body = "x".repeat(1025);
    let max_body = "é".repeat(512);
    let cases = [
        (json!({"Body": ""}), json!("InvalidParameterValue")),
        (json!({"Body": long_body}), json!("InvalidParameterValue")),
        (json!({"Body": 5}), json!("InvalidParameterValue")),
        (json!({"Body": max_body}), json!(null)),
        (
            json!({"Body": "x", "GroupId": "g"}),
            json!("UnsupportedOperation"),
        ),
        (
            json!({"Body": "x", "DeduplicationId": "d"}),
            json!("UnsupportedOperation"),
        ),
        (
            json!({"Body": "x", "GroupId": "not valid"}),
            json!("InvalidParameterValue"),
        ),
        (
            json!({"Body": "x", "DeduplicationId": ""}),
            json!("InvalidParameterValue"),
        ),
    ];
    for (mut extra, expected) in cases {
        let command = extra.as_object_mut().expect("cases are objects");
        command.insert("Command".into(), json!("SendMessage"));
        command.insert("QueueName".into(), json!("q"));
        let responses = exchange(&[create("q", json!({})), Value::Object(command.clone())]);
        assert_eq!(*code(&responses[1]), expected, "{extra}");
    }
}

#[test]
fn fifo_sends_need_a_group_and_a_deduplication_source() {
    let responses = exchange(&[
        create("q.fifo", json!({"Fifo": true})),
        create(
            "c.fifo",
            json!({"Fifo": true, "ContentBasedDeduplication": true}),
        ),
        json!({"Command": "SendMessage", "QueueName": "q.fifo", "Body": "x", "DelaySeconds": 1}),
        json!({"Command": "SendMessage", "QueueName": "q.fifo", "Body": "x", "DeduplicationId": "d"}),
        json!({"Command": "SendMessage", "QueueName": "q.fifo", "Body": "x", "GroupId": "g"}),
        json!({"Command": "SendMessage", "QueueName": "c.fifo", "Body": "x", "GroupId": "g"}),
        json!({"Command": "SendMessage", "QueueName": "q.fifo", "Body": "x", "GroupId": "g", "DeduplicationId": "d"}),
    ]);
    let codes: Vec<_> = responses[2..5].iter().map(|r| code(r).clone()).collect();
    assert_eq!(
        codes,
        [
            json!("UnsupportedOperation"),
            json!("MissingParameter"),
            json!("MissingParameter"),
        ]
    );
    assert_eq!(
        responses[5],
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
    assert_eq!(
        responses[6],
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 1})
    );
}

#[test]
fn a_duplicate_fifo_send_answers_with_the_original_and_the_window_does_not_extend() {
    let send_dup = |group: &str| {
        json!({"Command": "SendMessage", "QueueName": "q.fifo", "Body": "x",
               "GroupId": group, "DeduplicationId": "d"})
    };
    let responses = exchange(&[
        create("q.fifo", json!({"Fifo": true})),
        send_dup("g1"),
        tick(299),
        send_dup("g2"),
        tick(1),
        send_dup("g2"),
        send_dup("g1"),
        json!({"Command": "GetQueueAttributes", "QueueName": "q.fifo"}),
    ]);
    let first = json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1});
    let second = json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2});
    assert_eq!(responses[1], first);
    assert_eq!(responses[3], first);
    assert_eq!(responses[5], second);
    assert_eq!(responses[6], second);
    assert_eq!(
        responses[7]["Counts"],
        json!({"Visible": 2, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn deduplication_outlives_the_message_and_content_based_ids_use_the_body() {
    let responses = exchange(&[
        create(
            "q.fifo",
            json!({"Fifo": true, "ContentBasedDeduplication": true}),
        ),
        json!({"Command": "SendMessage", "QueueName": "q.fifo", "Body": "same", "GroupId": "g"}),
        receive("q.fifo"),
        json!({"Command": "DeleteMessage", "QueueName": "q.fifo", "ReceiptHandle": "r-1"}),
        json!({"Command": "SendMessage", "QueueName": "q.fifo", "Body": "same", "GroupId": "g"}),
        json!({"Command": "PurgeQueue", "QueueName": "q.fifo"}),
        json!({"Command": "SendMessage", "QueueName": "q.fifo", "Body": "same", "GroupId": "h"}),
        json!({"Command": "SendMessage", "QueueName": "q.fifo", "Body": "other", "GroupId": "g"}),
        json!({"Command": "SendMessage", "QueueName": "q.fifo", "Body": "x", "GroupId": "g", "DeduplicationId": "same"}),
    ]);
    let original = json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1});
    assert_eq!(responses[4], original);
    assert_eq!(responses[6], original);
    assert_eq!(
        responses[7],
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2})
    );
    assert_eq!(responses[8], original);
}

#[test]
fn a_fifo_receive_delivers_one_message_per_group_and_blocks_later_ones() {
    let fifo_send = |body: &str, group: &str, id: &str| {
        json!({"Command": "SendMessage", "QueueName": "q.fifo", "Body": body,
               "GroupId": group, "DeduplicationId": id})
    };
    let many = json!({"Command": "ReceiveMessage", "QueueName": "q.fifo", "MaxMessages": 10});
    let responses = exchange(&[
        create("q.fifo", json!({"Fifo": true})),
        fifo_send("a1", "a", "1"),
        fifo_send("a2", "a", "2"),
        fifo_send("b1", "b", "3"),
        many.clone(),
        many.clone(),
        json!({"Command": "DeleteMessage", "QueueName": "q.fifo", "ReceiptHandle": "r-1"}),
        many.clone(),
    ]);
    let fifo_message = |id: &str, handle: &str, body: &str, group: &str, sequence: u32| {
        json!({
            "MessageId": id, "ReceiptHandle": handle, "Body": body, "ReceiveCount": 1,
            "SentAt": 0, "FirstReceivedAt": 0, "GroupId": group, "SequenceNumber": sequence,
        })
    };
    assert_eq!(
        responses[4],
        json!({"ok": true, "Messages": [
            fifo_message("m-1", "r-1", "a1", "a", 1),
            fifo_message("m-3", "r-2", "b1", "b", 3),
        ]})
    );
    assert_eq!(responses[5], json!({"ok": true, "Messages": []}));
    assert_eq!(
        responses[7],
        json!({"ok": true, "Messages": [fifo_message("m-2", "r-3", "a2", "a", 2)]})
    );
}

#[test]
fn a_delayed_group_head_blocks_its_group() {
    let responses = exchange(&[
        create("q.fifo", json!({"Fifo": true, "DelaySeconds": 5})),
        json!({"Command": "SendMessage", "QueueName": "q.fifo", "Body": "a1", "GroupId": "a", "DeduplicationId": "1"}),
        json!({"Command": "SetQueueAttributes", "QueueName": "q.fifo", "Attributes": {"DelaySeconds": 0}}),
        json!({"Command": "SendMessage", "QueueName": "q.fifo", "Body": "a2", "GroupId": "a", "DeduplicationId": "2"}),
        json!({"Command": "SendMessage", "QueueName": "q.fifo", "Body": "b1", "GroupId": "b", "DeduplicationId": "3"}),
        json!({"Command": "ReceiveMessage", "QueueName": "q.fifo", "MaxMessages": 10}),
    ]);
    let delivered: Vec<_> = responses[5]["Messages"]
        .as_array()
        .expect("a receive result has a Messages array")
        .iter()
        .map(|m| m["Body"].clone())
        .collect();
    assert_eq!(delivered, [json!("b1")]);
}

#[test]
fn a_message_received_too_often_moves_to_the_dead_letter_queue() {
    let responses = exchange(&[
        create("dead", json!({})),
        create(
            "q",
            json!({"VisibilityTimeout": 0, "DeadLetterQueue": "dead", "MaxReceiveCount": 2}),
        ),
        send("q", "x"),
        send("q", "y"),
        json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 1}),
        json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 1}),
        counts("dead"),
        json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 1}),
        counts("dead"),
        receive("dead"),
        counts("q"),
    ]);
    assert_eq!(
        responses[4],
        json!({"ok": true, "Messages": [message("m-1", "r-1", "x", 1, 0, 0)]})
    );
    assert_eq!(
        responses[5],
        json!({"ok": true, "Messages": [message("m-1", "r-2", "x", 2, 0, 0)]})
    );
    assert_eq!(
        responses[6]["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(
        responses[7],
        json!({"ok": true, "Messages": [message("m-2", "r-3", "y", 1, 0, 0)]})
    );
    assert_eq!(
        responses[8]["Counts"],
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(
        responses[9],
        json!({"ok": true, "Messages": [message("m-1", "r-4", "x", 1, 0, 0)]})
    );
}

#[test]
fn a_dead_lettered_message_keeps_its_age_under_the_new_retention() {
    let responses = exchange(&[
        create("dead", json!({"RetentionSeconds": 60})),
        create(
            "q",
            json!({"VisibilityTimeout": 0, "DeadLetterQueue": "dead", "MaxReceiveCount": 1}),
        ),
        send("q", "x"),
        tick(100),
        receive("q"),
        receive("q"),
        counts("dead"),
    ]);
    assert_eq!(responses[5], json!({"ok": true, "Messages": []}));
    assert_eq!(
        responses[6]["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn a_fifo_dead_letter_move_takes_a_new_sequence_number_and_unblocks_the_group() {
    let fifo_send = |body: &str, id: &str| {
        json!({"Command": "SendMessage", "QueueName": "q.fifo", "Body": body,
               "GroupId": "g", "DeduplicationId": id})
    };
    let receive_all =
        json!({"Command": "ReceiveMessage", "QueueName": "q.fifo", "MaxMessages": 10});
    let responses = exchange(&[
        create("dead.fifo", json!({"Fifo": true})),
        create(
            "q.fifo",
            json!({"Fifo": true, "VisibilityTimeout": 0,
                                "DeadLetterQueue": "dead.fifo", "MaxReceiveCount": 1}),
        ),
        fifo_send("a1", "1"),
        fifo_send("a2", "2"),
        receive_all.clone(),
        receive_all,
        json!({"Command": "ReceiveMessage", "QueueName": "dead.fifo"}),
    ]);
    let delivered = |response: &Value| {
        response["Messages"]
            .as_array()
            .expect("a receive result has a Messages array")
            .iter()
            .map(|m| {
                (
                    m["Body"].clone(),
                    m["SequenceNumber"].clone(),
                    m["ReceiveCount"].clone(),
                )
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(
        delivered(&responses[4]),
        [(json!("a1"), json!(1), json!(1))]
    );
    assert_eq!(
        delivered(&responses[5]),
        [(json!("a2"), json!(2), json!(1))]
    );
    assert_eq!(
        delivered(&responses[6]),
        [(json!("a1"), json!(1), json!(1))]
    );
    assert_eq!(responses[6]["Messages"][0]["GroupId"], json!("g"));
}
