mod common;

use common::*;
use serde_json::{Value, json};

fn delete(queue: &str, handle: &str) -> Value {
    cmd(
        "DeleteMessage",
        json!({"QueueName": queue, "ReceiptHandle": handle}),
    )
}

fn change(queue: &str, handle: &str, timeout: u64) -> Value {
    cmd(
        "ChangeMessageVisibility",
        json!({"QueueName": queue, "ReceiptHandle": handle, "VisibilityTimeout": timeout}),
    )
}

fn receive_up_to(queue: &str, max: u64) -> Value {
    cmd(
        "ReceiveMessage",
        json!({"QueueName": queue, "MaxMessages": max}),
    )
}

fn visible_in_flight_delayed(response: &Value) -> Vec<u64> {
    let counts = &response["Counts"];
    ["Visible", "InFlight", "Delayed"]
        .iter()
        .map(|k| counts[k].as_u64().unwrap())
        .collect()
}

#[test]
fn message_ids_count_accepted_messages_over_all_queues() {
    let responses = run(&[
        create("a"),
        create("b"),
        send("a", "x"),
        send("missing", "x"),
        send("b", "y"),
        cmd("SendMessage", json!({"QueueName": "a", "Body": ""})),
        send("a", "z"),
    ]);
    assert_eq!(responses[2]["MessageId"], json!("m-1"));
    assert_eq!(responses[4]["MessageId"], json!("m-2"));
    assert_eq!(responses[6]["MessageId"], json!("m-3"));
}

#[test]
fn receive_delivers_in_arrival_order_and_hides_messages_for_the_timeout() {
    let responses = run(&[
        create("q"),
        send("q", "a"),
        send("q", "b"),
        tick(4),
        receive("q"),
        receive("q"),
        receive("q"),
        tick(30),
        receive_up_to("q", 10),
    ]);
    assert_eq!(
        responses[4],
        json!({"ok": true, "Messages": [{
            "MessageId": "m-1", "ReceiptHandle": "r-1", "Body": "a",
            "ReceiveCount": 1, "SentAt": 0, "FirstReceivedAt": 4
        }]})
    );
    assert_eq!(bodies(&responses[5]), ["b"]);
    assert_eq!(responses[5]["Messages"][0]["ReceiptHandle"], json!("r-2"));
    assert_eq!(bodies(&responses[6]), Vec::<&str>::new());
    // a returns to its place before b, with a new handle and count.
    let again = &responses[8]["Messages"];
    assert_eq!(again[0]["Body"], json!("a"));
    assert_eq!(again[0]["ReceiptHandle"], json!("r-3"));
    assert_eq!(again[0]["ReceiveCount"], json!(2));
    assert_eq!(again[0]["FirstReceivedAt"], json!(4));
    assert_eq!(again[1]["Body"], json!("b"));
}

#[test]
fn receive_honours_max_messages_and_a_custom_visibility_timeout() {
    let responses = run(&[
        create("q"),
        send("q", "a"),
        send("q", "b"),
        send("q", "c"),
        cmd(
            "ReceiveMessage",
            json!({"QueueName": "q", "MaxMessages": 2, "VisibilityTimeout": 5}),
        ),
        tick(5),
        receive_up_to("q", 10),
        receive_up_to("q", 0),
        receive_up_to("q", 11),
    ]);
    assert_eq!(bodies(&responses[4]), ["a", "b"]);
    assert_eq!(bodies(&responses[6]), ["a", "b", "c"]);
    assert_eq!(code(&responses[7]), "InvalidParameterValue");
    assert_eq!(code(&responses[8]), "InvalidParameterValue");
}

#[test]
fn a_zero_visibility_timeout_makes_the_message_visible_at_once_and_its_handle_useless() {
    let responses = run(&[
        create("q"),
        send("q", "a"),
        cmd(
            "ReceiveMessage",
            json!({"QueueName": "q", "VisibilityTimeout": 0}),
        ),
        delete("q", "r-1"),
        receive("q"),
    ]);
    assert_eq!(bodies(&responses[2]), ["a"]);
    assert_eq!(code(&responses[3]), "ReceiptHandleExpired");
    assert_eq!(responses[4]["Messages"][0]["ReceiveCount"], json!(2));
}

#[test]
fn delay_hides_a_message_until_its_time_and_the_per_message_delay_wins() {
    let responses = run(&[
        create_with("q", json!({"DelaySeconds": 10})),
        send("q", "queue-delay"),
        cmd(
            "SendMessage",
            json!({"QueueName": "q", "Body": "own", "DelaySeconds": 0}),
        ),
        cmd(
            "SendMessage",
            json!({"QueueName": "q", "Body": "long", "DelaySeconds": 900}),
        ),
        counts("q"),
        receive_up_to("q", 10),
        tick(9),
        counts("q"),
        tick(1),
        counts("q"),
        tick(890),
        counts("q"),
        cmd(
            "SendMessage",
            json!({"QueueName": "q", "Body": "x", "DelaySeconds": 901}),
        ),
    ]);
    assert_eq!(visible_in_flight_delayed(&responses[4]), [1, 0, 2]);
    assert_eq!(bodies(&responses[5]), ["own"]);
    assert_eq!(visible_in_flight_delayed(&responses[7]), [0, 1, 2]);
    assert_eq!(visible_in_flight_delayed(&responses[9]), [1, 1, 1]);
    assert_eq!(visible_in_flight_delayed(&responses[11]), [3, 0, 0]);
    assert_eq!(code(&responses[12]), "InvalidParameterValue");
}

#[test]
fn delete_and_change_visibility_distinguish_invalid_from_expired_handles() {
    let responses = run(&[
        create("q"),
        create("other"),
        send("q", "a"),
        send("other", "b"),
        receive("q"),
        receive("other"),
        delete("q", "r-9"),
        delete("q", "r-01"),
        delete("q", "R-1"),
        delete("q", "r-1 "),
        delete("q", "r-2"),
        change("q", "r-2", 10),
        delete("missing", "r-1"),
        delete("q", "r-1"),
        delete("q", "r-1"),
        change("q", "r-1", 1),
        cmd(
            "ChangeMessageVisibility",
            json!({"QueueName": "q", "ReceiptHandle": "r-2"}),
        ),
        change("q", "r-2", 43201),
    ]);
    let codes: Vec<_> = responses[6..].iter().map(outcome).collect();
    assert_eq!(
        codes,
        [
            "ReceiptHandleInvalid",
            "ReceiptHandleInvalid",
            "ReceiptHandleInvalid",
            "ReceiptHandleInvalid",
            "ReceiptHandleExpired",
            "ReceiptHandleExpired",
            "QueueDoesNotExist",
            "ok",
            "ReceiptHandleExpired",
            "ReceiptHandleExpired",
            "MissingParameter",
            "InvalidParameterValue"
        ]
    );
}

#[test]
fn only_the_latest_handle_of_a_message_works() {
    let responses = run(&[
        create("q"),
        send("q", "a"),
        cmd(
            "ReceiveMessage",
            json!({"QueueName": "q", "VisibilityTimeout": 5}),
        ),
        tick(5),
        receive("q"),
        delete("q", "r-1"),
        delete("q", "r-2"),
        counts("q"),
    ]);
    assert_eq!(code(&responses[5]), "ReceiptHandleExpired");
    assert_eq!(responses[6], ok());
    assert_eq!(visible_in_flight_delayed(&responses[7]), [0, 0, 0]);
}

#[test]
fn change_visibility_replaces_the_end_of_the_timeout_without_counting_a_receive() {
    let responses = run(&[
        create("q"),
        send("q", "a"),
        receive("q"),
        change("q", "r-1", 100),
        tick(99),
        counts("q"),
        tick(1),
        counts("q"),
        receive("q"),
        change("q", "r-2", 0),
        counts("q"),
        delete("q", "r-2"),
    ]);
    assert_eq!(visible_in_flight_delayed(&responses[5]), [0, 1, 0]);
    assert_eq!(visible_in_flight_delayed(&responses[7]), [1, 0, 0]);
    assert_eq!(responses[8]["Messages"][0]["ReceiveCount"], json!(2));
    assert_eq!(visible_in_flight_delayed(&responses[10]), [1, 0, 0]);
    assert_eq!(code(&responses[11]), "ReceiptHandleExpired");
}

#[test]
fn a_shortened_timeout_makes_the_message_visible_earlier() {
    let responses = run(&[
        create("q"),
        send("q", "a"),
        receive("q"),
        change("q", "r-1", 2),
        tick(2),
        receive("q"),
    ]);
    assert_eq!(bodies(&responses[5]), ["a"]);
}

#[test]
fn retention_removes_messages_in_every_state_and_is_not_undone() {
    let set = |seconds: u64| {
        cmd(
            "SetQueueAttributes",
            json!({"QueueName": "q", "Attributes": {"RetentionSeconds": seconds}}),
        )
    };
    let responses = run(&[
        create_with("q", json!({"RetentionSeconds": 100})),
        send("q", "a"),
        tick(30),
        send("q", "b"),
        cmd("SendMessage", json!({"QueueName": "q", "Body": "c"})),
        receive("q"),
        tick(29),
        counts("q"),
        tick(41),
        counts("q"),
        // Lowering removes at once; raising later brings nothing back.
        set(70),
        counts("q"),
        set(1000),
        counts("q"),
        send("q", "d"),
        tick(30),
        set(1000),
        counts("q"),
    ]);
    assert_eq!(visible_in_flight_delayed(&responses[7]), [2, 1, 0]);
    assert_eq!(visible_in_flight_delayed(&responses[9]), [2, 0, 0]);
    assert_eq!(visible_in_flight_delayed(&responses[11]), [0, 0, 0]);
    assert_eq!(visible_in_flight_delayed(&responses[13]), [0, 0, 0]);
    assert_eq!(visible_in_flight_delayed(&responses[17]), [1, 0, 0]);
}

#[test]
fn raising_retention_after_a_quiet_period_does_not_resurrect_expired_messages() {
    let responses = run(&[
        create_with("q", json!({"RetentionSeconds": 60})),
        send("q", "a"),
        tick(100),
        cmd(
            "SetQueueAttributes",
            json!({"QueueName": "q", "Attributes": {"RetentionSeconds": 1000}}),
        ),
        counts("q"),
    ]);
    assert_eq!(visible_in_flight_delayed(&responses[4]), [0, 0, 0]);
}

#[test]
fn standard_queues_reject_fifo_parameters() {
    let responses = run(&[
        create("q"),
        cmd(
            "SendMessage",
            json!({"QueueName": "q", "Body": "x", "GroupId": "g"}),
        ),
        cmd(
            "SendMessage",
            json!({"QueueName": "q", "Body": "x", "DeduplicationId": "d"}),
        ),
        cmd(
            "SendMessage",
            json!({"QueueName": "q", "Body": "x", "GroupId": "bad group"}),
        ),
    ]);
    let codes: Vec<_> = responses[1..].iter().map(outcome).collect();
    assert_eq!(
        codes,
        [
            "UnsupportedOperation",
            "UnsupportedOperation",
            "InvalidParameterValue"
        ]
    );
}

fn fifo_send(queue: &str, body: &str, group: &str, dedup: &str) -> Value {
    cmd(
        "SendMessage",
        json!({"QueueName": queue, "Body": body, "GroupId": group, "DeduplicationId": dedup}),
    )
}

#[test]
fn fifo_send_rules_apply_in_the_documented_order() {
    let responses = run(&[
        create_with("f.fifo", json!({"Fifo": true})),
        create_with(
            "c.fifo",
            json!({"Fifo": true, "ContentBasedDeduplication": true}),
        ),
        cmd(
            "SendMessage",
            json!({"QueueName": "f.fifo", "Body": "x", "DelaySeconds": 1}),
        ),
        cmd("SendMessage", json!({"QueueName": "f.fifo", "Body": "x"})),
        cmd(
            "SendMessage",
            json!({"QueueName": "f.fifo", "Body": "x", "GroupId": "g"}),
        ),
        cmd(
            "SendMessage",
            json!({"QueueName": "c.fifo", "Body": "x", "GroupId": "g"}),
        ),
        fifo_send("f.fifo", "x", "g", "d"),
    ]);
    let results: Vec<_> = responses[2..].iter().map(outcome).collect();
    assert_eq!(
        results,
        [
            "UnsupportedOperation",
            "MissingParameter",
            "MissingParameter",
            "ok",
            "ok"
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
fn a_queue_default_delay_applies_to_fifo_sends() {
    let responses = run(&[
        create_with("f.fifo", json!({"Fifo": true, "DelaySeconds": 5})),
        fifo_send("f.fifo", "x", "g", "d"),
        counts("f.fifo"),
    ]);
    assert_eq!(visible_in_flight_delayed(&responses[2]), [0, 0, 1]);
}

#[test]
fn duplicate_sends_return_the_original_and_use_no_message_id() {
    let responses = run(&[
        create_with("f.fifo", json!({"Fifo": true})),
        fifo_send("f.fifo", "a", "g", "d"),
        tick(299),
        fifo_send("f.fifo", "other body", "other-group", "d"),
        fifo_send("f.fifo", "b", "g", "e"),
        tick(1),
        // The duplicate at 299 did not extend the window: 300 is past it.
        fifo_send("f.fifo", "again", "g", "d"),
        fifo_send("f.fifo", "again", "g", "d"),
    ]);
    assert_eq!(
        responses[3],
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
    assert_eq!(
        responses[4],
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2})
    );
    assert_eq!(
        responses[6],
        json!({"ok": true, "MessageId": "m-3", "SequenceNumber": 3})
    );
    assert_eq!(responses[7], responses[6]);
}

#[test]
fn deduplication_outlives_the_message_and_ignores_the_group() {
    let responses = run(&[
        create_with("f.fifo", json!({"Fifo": true})),
        fifo_send("f.fifo", "a", "g", "d"),
        receive("f.fifo"),
        delete("f.fifo", "r-1"),
        fifo_send("f.fifo", "a", "g2", "d"),
        cmd("PurgeQueue", json!({"QueueName": "f.fifo"})),
        fifo_send("f.fifo", "a", "g", "d"),
        fifo_send("f.fifo", "a", "g", "new"),
    ]);
    assert_eq!(responses[4]["MessageId"], json!("m-1"));
    assert_eq!(responses[6]["MessageId"], json!("m-1"));
    // Sequence numbers carry on after a purge.
    assert_eq!(
        responses[7],
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2})
    );
}

#[test]
fn content_based_deduplication_uses_the_body_as_id() {
    let body = "b".repeat(1024);
    let responses = run(&[
        create_with(
            "c.fifo",
            json!({"Fifo": true, "ContentBasedDeduplication": true}),
        ),
        cmd(
            "SendMessage",
            json!({"QueueName": "c.fifo", "Body": body, "GroupId": "g"}),
        ),
        cmd(
            "SendMessage",
            json!({"QueueName": "c.fifo", "Body": body, "GroupId": "h"}),
        ),
        cmd(
            "SendMessage",
            json!({"QueueName": "c.fifo", "Body": "short", "GroupId": "g"}),
        ),
        // An explicit id is a different key than the same text as a body.
        cmd(
            "SendMessage",
            json!({"QueueName": "c.fifo", "Body": "x", "GroupId": "g", "DeduplicationId": "short"}),
        ),
    ]);
    assert_eq!(responses[2]["MessageId"], json!("m-1"));
    assert_eq!(responses[3]["MessageId"], json!("m-2"));
    assert_eq!(responses[4]["MessageId"], json!("m-2"));
}

#[test]
fn fifo_receive_delivers_one_message_per_group_and_blocks_busy_groups() {
    let responses = run(&[
        create_with("f.fifo", json!({"Fifo": true})),
        fifo_send("f.fifo", "a1", "A", "1"),
        fifo_send("f.fifo", "a2", "A", "2"),
        fifo_send("f.fifo", "b1", "B", "3"),
        fifo_send("f.fifo", "a3", "A", "4"),
        receive_up_to("f.fifo", 10),
        receive_up_to("f.fifo", 10),
        delete("f.fifo", "r-1"),
        receive_up_to("f.fifo", 10),
    ]);
    assert_eq!(bodies(&responses[5]), ["a1", "b1"]);
    assert_eq!(
        responses[5]["Messages"][0],
        json!({
            "MessageId": "m-1", "ReceiptHandle": "r-1", "Body": "a1", "ReceiveCount": 1,
            "SentAt": 0, "FirstReceivedAt": 0, "GroupId": "A", "SequenceNumber": 1
        })
    );
    assert_eq!(bodies(&responses[6]), Vec::<&str>::new());
    assert_eq!(bodies(&responses[8]), ["a2"]);
}

#[test]
fn a_delayed_head_blocks_its_group_on_a_queue_whose_delay_was_lowered() {
    let responses = run(&[
        create_with("f.fifo", json!({"Fifo": true, "DelaySeconds": 10})),
        fifo_send("f.fifo", "a1", "A", "1"),
        cmd(
            "SetQueueAttributes",
            json!({"QueueName": "f.fifo", "Attributes": {"DelaySeconds": 0}}),
        ),
        fifo_send("f.fifo", "a2", "A", "2"),
        fifo_send("f.fifo", "b1", "B", "3"),
        receive_up_to("f.fifo", 10),
        tick(10),
        receive_up_to("f.fifo", 10),
    ]);
    assert_eq!(bodies(&responses[5]), ["b1"]);
    assert_eq!(bodies(&responses[7]), ["a1"]);
}

fn dead_letter_scenario(fifo: bool) -> Vec<Value> {
    let suffix = if fifo { ".fifo" } else { "" };
    let (dlq, main) = (format!("dlq{suffix}"), format!("main{suffix}"));
    let send_to = |queue: &str, body: &str, id: &str| {
        if fifo {
            fifo_send(queue, body, "g", id)
        } else {
            send(queue, body)
        }
    };
    run(&[
        create_with(&dlq, json!({"Fifo": fifo, "RetentionSeconds": 100})),
        create_with(
            &main,
            json!({"Fifo": fifo, "VisibilityTimeout": 1, "DeadLetterQueue": dlq, "MaxReceiveCount": 2}),
        ),
        send_to(&dlq, "existing", "x"),
        send_to(&main, "poison", "y"),
        receive(&main),
        tick(1),
        receive(&main),
        tick(1),
        receive(&main),
        counts(&main),
        receive_up_to(&dlq, 10),
        receive_up_to(&dlq, 10),
    ])
}

#[test]
fn a_message_over_max_receive_count_moves_to_the_dead_letter_queue() {
    let responses = dead_letter_scenario(false);
    assert_eq!(responses[6]["Messages"][0]["ReceiveCount"], json!(2));
    assert_eq!(bodies(&responses[8]), Vec::<&str>::new());
    assert_eq!(visible_in_flight_delayed(&responses[9]), [0, 0, 0]);
    // The dead-letter queue holds its own message first, then the moved one
    // with its original id, a reset count and no first-receive time.
    assert_eq!(bodies(&responses[10]), ["existing", "poison"]);
    assert_eq!(
        responses[10]["Messages"][1],
        json!({
            "MessageId": "m-2", "ReceiptHandle": "r-4", "Body": "poison",
            "ReceiveCount": 1, "SentAt": 0, "FirstReceivedAt": 2
        })
    );
}

#[test]
fn dead_letter_moves_in_fifo_queues_take_the_next_sequence_number() {
    let responses = dead_letter_scenario(true);
    assert_eq!(bodies(&responses[8]), Vec::<&str>::new());
    assert_eq!(bodies(&responses[10]), ["existing"]);
    // Both messages share group g, so the moved one waits behind the first.
    assert_eq!(responses[10]["Messages"][0]["SequenceNumber"], json!(1));
    assert_eq!(bodies(&responses[11]), Vec::<&str>::new());
}

#[test]
fn a_moved_message_that_is_already_too_old_is_dropped_on_arrival() {
    let responses = run(&[
        create_with("dlq", json!({"RetentionSeconds": 60})),
        create_with(
            "main",
            json!({"VisibilityTimeout": 1, "DeadLetterQueue": "dlq", "MaxReceiveCount": 1}),
        ),
        send("main", "old"),
        receive("main"),
        tick(60),
        receive("main"),
        counts("dlq"),
    ]);
    assert_eq!(bodies(&responses[5]), Vec::<&str>::new());
    assert_eq!(visible_in_flight_delayed(&responses[6]), [0, 0, 0]);
}

#[test]
fn a_dead_letter_move_does_not_count_toward_max_messages_and_keeps_examining() {
    let responses = run(&[
        create("dlq"),
        create_with(
            "main",
            json!({"VisibilityTimeout": 0, "DeadLetterQueue": "dlq", "MaxReceiveCount": 1}),
        ),
        send("main", "a"),
        receive("main"),
        send("main", "b"),
        send("main", "c"),
        receive_up_to("main", 2),
        counts("dlq"),
    ]);
    assert_eq!(bodies(&responses[6]), ["b", "c"]);
    assert_eq!(visible_in_flight_delayed(&responses[7]), [1, 0, 0]);
}

#[test]
fn a_new_dead_letter_configuration_applies_from_the_next_receive() {
    let responses = run(&[
        create("dlq"),
        create_with("q", json!({"VisibilityTimeout": 0})),
        send("q", "a"),
        receive("q"),
        receive("q"),
        cmd(
            "SetQueueAttributes",
            json!({"QueueName": "q", "Attributes": {"DeadLetterQueue": "dlq", "MaxReceiveCount": 2}}),
        ),
        receive("q"),
        counts("dlq"),
    ]);
    assert_eq!(bodies(&responses[6]), Vec::<&str>::new());
    assert_eq!(visible_in_flight_delayed(&responses[7]), [1, 0, 0]);
}

#[test]
fn a_queue_holds_at_most_100000_messages_and_fills_quickly() {
    let mut commands = vec![create("q")];
    commands.extend((0..100_000).map(|_| send("q", "x")));
    commands.push(send("q", "one too many"));
    commands.push(cmd("PurgeQueue", json!({"QueueName": "q"})));
    commands.push(send("q", "after purge"));
    let started = std::time::Instant::now();
    let responses = run(&commands);
    assert!(started.elapsed() < std::time::Duration::from_secs(20));
    assert_eq!(
        responses[100_000],
        json!({"ok": true, "MessageId": "m-100000"})
    );
    assert_eq!(code(&responses[100_001]), "LimitExceeded");
    assert_eq!(
        responses[100_003],
        json!({"ok": true, "MessageId": "m-100001"})
    );
}

#[test]
fn receiving_from_a_long_queue_in_batches_stays_fast() {
    let mut commands = vec![create("q")];
    commands.extend((0..20_000).map(|_| send("q", "x")));
    commands.extend((0..2_000).map(|_| receive_up_to("q", 10)));
    let started = std::time::Instant::now();
    let responses = run(&commands);
    assert!(started.elapsed() < std::time::Duration::from_secs(20));
    let handles: Vec<u64> = responses[20_001..]
        .iter()
        .flat_map(|r| r["Messages"].as_array().unwrap())
        .map(|m| m["ReceiptHandle"].as_str().unwrap()[2..].parse().unwrap())
        .collect();
    assert_eq!(handles, (1..=20_000).collect::<Vec<u64>>());
}
