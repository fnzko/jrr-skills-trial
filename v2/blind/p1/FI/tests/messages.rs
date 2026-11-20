mod common;

use common::*;
use serde_json::json;

#[test]
fn standard_queue_round_trip_reports_the_whole_delivery() {
    let responses = run_values(&[
        create("q", json!({})),
        send("q", "first"),
        tick(5),
        send("q", "second"),
        receive_many("q", 10),
    ]);
    assert_eq!(responses[1], json!({"ok": true, "MessageId": "m-1"}));
    assert_eq!(responses[3], json!({"ok": true, "MessageId": "m-2"}));
    assert_eq!(
        responses[4],
        json!({
            "ok": true,
            "Messages": [
                {"MessageId": "m-1", "ReceiptHandle": "r-1", "Body": "first",
                 "ReceiveCount": 1, "SentAt": 0, "FirstReceivedAt": 5},
                {"MessageId": "m-2", "ReceiptHandle": "r-2", "Body": "second",
                 "ReceiveCount": 1, "SentAt": 5, "FirstReceivedAt": 5},
            ]
        })
    );
}

#[test]
fn fifo_queue_deliveries_carry_group_and_sequence_number() {
    let responses = run_values(&[
        create("f.fifo", json!({"Fifo": true})),
        json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "b", "GroupId": "g", "DeduplicationId": "d"}),
        receive("f.fifo"),
    ]);
    assert_eq!(
        responses[1],
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
    assert_eq!(
        responses[2]["Messages"],
        json!([{"MessageId": "m-1", "ReceiptHandle": "r-1", "Body": "b", "ReceiveCount": 1,
                "SentAt": 0, "FirstReceivedAt": 0, "GroupId": "g", "SequenceNumber": 1}])
    );
}

#[test]
fn receiving_from_an_empty_queue_returns_an_empty_array() {
    let responses = run_values(&[create("q", json!({})), receive("q")]);
    assert_eq!(responses[1], json!({"ok": true, "Messages": []}));
}

#[test]
fn bodies_can_hold_any_text_up_to_1024_bytes() {
    let body = "ü\"\\\n\t😀 {\"Command\":\"Tick\"}";
    let long = "é".repeat(512);
    let responses = run_values(&[
        create("q", json!({})),
        send("q", body),
        send("q", &long),
        send("q", &"x".repeat(1025)),
        receive_many("q", 10),
    ]);
    assert_eq!(responses[3]["ok"], false);
    assert_eq!(bodies(&responses[4]), [body, long.as_str()]);
}

#[test]
fn message_ids_count_accepted_messages_over_all_queues() {
    let responses = run_values(&[
        create("a", json!({})),
        create("b", json!({})),
        send("a", "1"),
        send("missing", "x"),
        send("b", "2"),
        json!({"Command": "SendMessage", "QueueName": "a", "Body": ""}),
        send("a", "3"),
    ]);
    let ids: Vec<_> = [2, 4, 6]
        .iter()
        .map(|&i| responses[i]["MessageId"].clone())
        .collect();
    assert_eq!(ids, [json!("m-1"), json!("m-2"), json!("m-3")]);
}

#[test]
fn receipt_handles_count_deliveries_over_all_queues() {
    let responses = run_values(&[
        create("a", json!({})),
        create("b", json!({})),
        send("a", "1"),
        send("b", "2"),
        receive("b"),
        receive("a"),
        tick(30),
        receive("a"),
    ]);
    assert_eq!(responses[4]["Messages"][0]["ReceiptHandle"], "r-1");
    assert_eq!(responses[5]["Messages"][0]["ReceiptHandle"], "r-2");
    assert_eq!(responses[7]["Messages"][0]["ReceiptHandle"], "r-3");
    assert_eq!(responses[7]["Messages"][0]["ReceiveCount"], 2);
    assert_eq!(responses[7]["Messages"][0]["FirstReceivedAt"], 0);
}

#[test]
fn send_message_rules_apply_in_the_documented_order() {
    let fifo_send = |extra: serde_json::Value| {
        let mut command = json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "b"});
        command
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        command
    };
    let std_send = |extra: serde_json::Value| {
        let mut command = json!({"Command": "SendMessage", "QueueName": "q", "Body": "b"});
        command
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        command
    };
    let responses = run_values(&[
        create("q", json!({})),
        create("f.fifo", json!({"Fifo": true})),
        std_send(json!({"GroupId": "g"})),
        std_send(json!({"DeduplicationId": "d"})),
        std_send(json!({"DelaySeconds": 3})),
        fifo_send(json!({"DelaySeconds": 3})),
        fifo_send(json!({"DelaySeconds": 3, "GroupId": "g", "DeduplicationId": "d"})),
        fifo_send(json!({"DeduplicationId": "d"})),
        fifo_send(json!({"GroupId": "g"})),
        fifo_send(json!({"GroupId": "g", "DeduplicationId": "d"})),
    ]);
    let codes: Vec<_> = responses[2..]
        .iter()
        .map(|r| if r["ok"] == true { "ok" } else { error_code(r) })
        .collect();
    assert_eq!(
        codes,
        [
            "UnsupportedOperation",
            "UnsupportedOperation",
            "ok",
            "UnsupportedOperation",
            "UnsupportedOperation",
            "MissingParameter",
            "MissingParameter",
            "ok",
        ]
    );
}

#[test]
fn delay_comes_from_the_parameter_or_else_the_queue_at_send_time() {
    let responses = run_values(&[
        create("q", json!({"DelaySeconds": 10})),
        send("q", "queue-delay"),
        json!({"Command": "SendMessage", "QueueName": "q", "Body": "no-delay", "DelaySeconds": 0}),
        json!({"Command": "SendMessage", "QueueName": "q", "Body": "long", "DelaySeconds": 20}),
        json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"DelaySeconds": 0}}),
        send("q", "after-change"),
        counts("q"),
        receive_many("q", 10),
        tick(10),
        counts("q"),
        receive_many("q", 10),
        tick(10),
        receive_many("q", 10),
    ]);
    assert_eq!(state_counts(&responses[6]), [2, 0, 2]);
    assert_eq!(bodies(&responses[7]), ["no-delay", "after-change"]);
    assert_eq!(state_counts(&responses[9]), [1, 2, 1]);
    assert_eq!(bodies(&responses[10]), ["queue-delay"]);
    assert_eq!(bodies(&responses[12]), ["long"]);
}

#[test]
fn received_message_is_hidden_for_the_visibility_timeout_then_returns_in_place() {
    let responses = run_values(&[
        create("q", json!({"VisibilityTimeout": 10})),
        send("q", "a"),
        send("q", "b"),
        receive("q"),
        tick(9),
        receive_many("q", 5),
        tick(1),
        receive_many("q", 5),
    ]);
    assert_eq!(bodies(&responses[3]), ["a"]);
    assert_eq!(bodies(&responses[5]), ["b"]);
    // At t=10 "a" is visible again, "b" (received at t=9) is not.
    assert_eq!(bodies(&responses[7]), ["a"]);
    assert_eq!(responses[7]["Messages"][0]["ReceiveCount"], 2);
}

#[test]
fn receive_can_override_the_visibility_timeout() {
    let responses = run_values(&[
        create("q", json!({})),
        send("q", "a"),
        json!({"Command": "ReceiveMessage", "QueueName": "q", "VisibilityTimeout": 2}),
        tick(1),
        counts("q"),
        tick(1),
        counts("q"),
    ]);
    assert_eq!(state_counts(&responses[4]), [0, 1, 0]);
    assert_eq!(state_counts(&responses[6]), [1, 0, 0]);
}

#[test]
fn zero_visibility_timeout_makes_the_message_visible_at_once_and_its_handle_useless() {
    let responses = run_values(&[
        create("q", json!({})),
        send("q", "a"),
        json!({"Command": "ReceiveMessage", "QueueName": "q", "VisibilityTimeout": 0}),
        delete("q", "r-1"),
        change_visibility("q", "r-1", 10),
        receive("q"),
    ]);
    assert_eq!(error_code(&responses[3]), "ReceiptHandleExpired");
    assert_eq!(error_code(&responses[4]), "ReceiptHandleExpired");
    assert_eq!(responses[5]["Messages"][0]["ReceiveCount"], 2);
    assert_eq!(responses[5]["Messages"][0]["ReceiptHandle"], "r-2");
}

#[test]
fn a_new_queue_visibility_timeout_applies_to_later_deliveries_only() {
    let responses = run_values(&[
        create("q", json!({"VisibilityTimeout": 100})),
        send("q", "a"),
        send("q", "b"),
        receive("q"),
        json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"VisibilityTimeout": 5}}),
        receive("q"),
        tick(5),
        counts("q"),
    ]);
    assert_eq!(state_counts(&responses[7]), [1, 1, 0]);
}

#[test]
fn max_messages_limits_a_receive_call() {
    let mut commands = vec![create("q", json!({}))];
    commands.extend((1..=5).map(|n| send("q", &n.to_string())));
    commands.push(receive_many("q", 3));
    commands.push(receive_many("q", 3));
    commands.push(receive("q"));
    let responses = run_values(&commands);
    assert_eq!(bodies(&responses[6]), ["1", "2", "3"]);
    assert_eq!(bodies(&responses[7]), ["4", "5"]);
    assert_eq!(bodies(&responses[8]), Vec::<&str>::new());
}

#[test]
fn messages_are_removed_once_their_retention_has_run_out() {
    let responses = run_values(&[
        create("q", json!({"RetentionSeconds": 60})),
        send("q", "old"),
        tick(30),
        send("q", "young"),
        tick(29),
        counts("q"),
        tick(1),
        counts("q"),
        tick(30),
        counts("q"),
    ]);
    assert_eq!(state_counts(&responses[5]), [2, 0, 0]);
    assert_eq!(state_counts(&responses[7]), [1, 0, 0]);
    assert_eq!(state_counts(&responses[9]), [0, 0, 0]);
}

#[test]
fn retention_removes_messages_in_any_state() {
    let responses = run_values(&[
        create("q", json!({"RetentionSeconds": 60})),
        send("q", "in-flight"),
        json!({"Command": "SendMessage", "QueueName": "q", "Body": "delayed", "DelaySeconds": 900}),
        send("q", "visible"),
        json!({"Command": "ReceiveMessage", "QueueName": "q", "VisibilityTimeout": 43200}),
        counts("q"),
        tick(60),
        counts("q"),
        delete("q", "r-1"),
    ]);
    assert_eq!(state_counts(&responses[5]), [1, 1, 1]);
    assert_eq!(state_counts(&responses[7]), [0, 0, 0]);
    assert_eq!(error_code(&responses[8]), "ReceiptHandleExpired");
}

#[test]
fn lowering_the_retention_removes_at_once_and_raising_it_brings_nothing_back() {
    let set_retention = |seconds: u64| json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"RetentionSeconds": seconds}});
    let responses = run_values(&[
        create("q", json!({})),
        send("q", "a"),
        tick(100),
        send("q", "b"),
        set_retention(60),
        counts("q"),
        set_retention(1000),
        counts("q"),
        tick(900),
        counts("q"),
    ]);
    assert_eq!(state_counts(&responses[5]), [1, 0, 0]);
    assert_eq!(state_counts(&responses[7]), [1, 0, 0]);
    // "b" was sent at 100 and now 1000 >= 100 + 1000 is false: still there at 1000, gone at 1100.
    assert_eq!(state_counts(&responses[9]), [1, 0, 0]);
}

#[test]
fn raising_the_retention_does_not_revive_messages_that_expired_in_the_meantime() {
    let responses = run_values(&[
        create("q", json!({"RetentionSeconds": 60})),
        send("q", "a"),
        tick(61),
        json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"RetentionSeconds": 1000}}),
        counts("q"),
    ]);
    assert_eq!(state_counts(&responses[4]), [0, 0, 0]);
}

#[test]
fn a_full_queue_refuses_sends_until_room_is_made() {
    let mut commands = vec![create("q", json!({}))];
    let line = |n: usize| format!(r#"{{"Command":"SendMessage","QueueName":"q","Body":"{n}"}}"#);
    let mut lines: Vec<String> = commands.drain(..).map(|c| c.to_string()).collect();
    lines.extend((0..100_000).map(line));
    lines.push(line(100_000));
    lines.push(r#"{"Command":"ReceiveMessage","QueueName":"q"}"#.to_owned());
    lines.push(r#"{"Command":"DeleteMessage","QueueName":"q","ReceiptHandle":"r-1"}"#.to_owned());
    lines.push(line(100_001));
    let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
    let responses = run(&lines);
    assert!(responses[1..=100_000].iter().all(|r| r["ok"] == true));
    assert_eq!(error_code(&responses[100_001]), "LimitExceeded");
    assert_eq!(responses[100_003], json!({"ok": true}));
    assert_eq!(
        responses[100_004],
        json!({"ok": true, "MessageId": "m-100001"})
    );
}

#[test]
fn fifo_delivers_one_message_per_group_in_arrival_order() {
    let send_to = |body: &str, group: &str| json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": body, "GroupId": group, "DeduplicationId": body});
    let responses = run_values(&[
        create("f.fifo", json!({"Fifo": true})),
        send_to("a1", "a"),
        send_to("a2", "a"),
        send_to("b1", "b"),
        send_to("a3", "a"),
        send_to("c1", "c"),
        receive_many("f.fifo", 10),
        receive_many("f.fifo", 10),
        delete("f.fifo", "r-1"),
        receive_many("f.fifo", 10),
    ]);
    assert_eq!(bodies(&responses[6]), ["a1", "b1", "c1"]);
    assert_eq!(bodies(&responses[7]), Vec::<&str>::new());
    assert_eq!(bodies(&responses[9]), ["a2"]);
}

#[test]
fn fifo_group_stays_blocked_while_its_earliest_message_is_in_flight_or_delayed() {
    let send_to = |body: &str, group: &str| json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": body, "GroupId": group, "DeduplicationId": body});
    let responses = run_values(&[
        create("f.fifo", json!({"Fifo": true, "DelaySeconds": 10})),
        send_to("a1", "a"),
        json!({"Command": "SetQueueAttributes", "QueueName": "f.fifo", "Attributes": {"DelaySeconds": 0}}),
        send_to("a2", "a"),
        send_to("b1", "b"),
        // a1 is delayed: a2 must wait for it, b1 is free.
        receive_many("f.fifo", 10),
        tick(10),
        receive_many("f.fifo", 10),
        // a1 is now in flight: a2 stays blocked.
        receive_many("f.fifo", 10),
        delete("f.fifo", "r-2"),
        receive_many("f.fifo", 10),
    ]);
    assert_eq!(bodies(&responses[5]), ["b1"]);
    assert_eq!(bodies(&responses[7]), ["a1"]);
    assert_eq!(bodies(&responses[8]), Vec::<&str>::new());
    assert_eq!(bodies(&responses[10]), ["a2"]);
}

#[test]
fn fifo_duplicate_returns_the_original_ids_and_changes_nothing() {
    let dup = |body: &str, group: &str| json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": body, "GroupId": group, "DeduplicationId": "same"});
    let responses = run_values(&[
        create("f.fifo", json!({"Fifo": true})),
        dup("first", "g1"),
        tick(299),
        dup("second", "g2"),
        send_to_other_fifo(),
        counts("f.fifo"),
    ]);
    assert_eq!(
        responses[1],
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
    assert_eq!(
        responses[3],
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
    // The duplicate used no message number.
    assert_eq!(responses[4]["MessageId"], "m-2");
    assert_eq!(state_counts(&responses[5]), [2, 0, 0]);
}

fn send_to_other_fifo() -> serde_json::Value {
    json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "x", "GroupId": "g", "DeduplicationId": "other"})
}

#[test]
fn deduplication_window_does_not_slide_and_reopens_after_300_seconds() {
    let dup = json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "b", "GroupId": "g", "DeduplicationId": "k"});
    let responses = run_values(&[
        create("f.fifo", json!({"Fifo": true})),
        dup.clone(),
        tick(200),
        dup.clone(),
        tick(100),
        dup.clone(),
        tick(299),
        dup.clone(),
        tick(1),
        dup,
    ]);
    // t=0 accepted; t=200 duplicate (does not extend); t=300 accepted anew;
    // t=599 duplicate of the second; t=600 accepted anew.
    let ids: Vec<_> = [1, 3, 5, 7, 9]
        .iter()
        .map(|&i| responses[i]["MessageId"].clone())
        .collect();
    assert_eq!(
        ids,
        [
            json!("m-1"),
            json!("m-1"),
            json!("m-2"),
            json!("m-2"),
            json!("m-3")
        ]
    );
    let sequences: Vec<_> = [1, 3, 5, 7, 9]
        .iter()
        .map(|&i| responses[i]["SequenceNumber"].clone())
        .collect();
    assert_eq!(
        sequences,
        [json!(1), json!(1), json!(2), json!(2), json!(3)]
    );
}

#[test]
fn content_based_deduplication_uses_the_body_and_an_explicit_id_wins() {
    let send_body = |body: &str, dedup: Option<&str>| {
        let mut command =
            json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": body, "GroupId": "g"});
        if let Some(id) = dedup {
            command["DeduplicationId"] = json!(id);
        }
        command
    };
    let responses = run_values(&[
        create(
            "f.fifo",
            json!({"Fifo": true, "ContentBasedDeduplication": true}),
        ),
        send_body("same body with spaces & symbols!", None),
        send_body("same body with spaces & symbols!", None),
        send_body("same body with spaces & symbols!", Some("explicit")),
        send_body("other", Some("explicit")),
        send_body("other", None),
    ]);
    let ids: Vec<_> = responses[1..]
        .iter()
        .map(|r| r["MessageId"].clone())
        .collect();
    assert_eq!(
        ids,
        [
            json!("m-1"),
            json!("m-1"),
            json!("m-2"),
            json!("m-2"),
            json!("m-3")
        ]
    );
}

#[test]
fn deduplication_ids_outlive_the_message_and_are_per_queue() {
    let to = |queue: &str| json!({"Command": "SendMessage", "QueueName": queue, "Body": "b", "GroupId": "g", "DeduplicationId": "k"});
    let responses = run_values(&[
        create("f.fifo", json!({"Fifo": true})),
        create("g.fifo", json!({"Fifo": true})),
        to("f.fifo"),
        receive("f.fifo"),
        delete("f.fifo", "r-1"),
        to("f.fifo"),
        to("g.fifo"),
    ]);
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
fn a_full_fifo_queue_still_answers_duplicates() {
    let line = |n: usize| {
        format!(
            r#"{{"Command":"SendMessage","QueueName":"f.fifo","Body":"{n}","GroupId":"g","DeduplicationId":"d{n}"}}"#
        )
    };
    let mut lines = vec![create("f.fifo", json!({"Fifo": true})).to_string()];
    lines.extend((0..100_000).map(line));
    lines.push(line(0));
    lines.push(line(100_000));
    let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
    let responses = run(&lines);
    assert_eq!(
        responses[100_001],
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
    assert_eq!(error_code(&responses[100_002]), "LimitExceeded");
}

#[test]
fn sequence_numbers_count_the_messages_added_to_the_queue() {
    let send_to = |queue: &str, n: u64| json!({"Command": "SendMessage", "QueueName": queue, "Body": "b", "GroupId": "g", "DeduplicationId": n.to_string()});
    let responses = run_values(&[
        create("f.fifo", json!({"Fifo": true})),
        create("g.fifo", json!({"Fifo": true})),
        send_to("f.fifo", 1),
        send_to("g.fifo", 2),
        send_to("f.fifo", 3),
    ]);
    assert_eq!(responses[2]["SequenceNumber"], 1);
    assert_eq!(responses[3]["SequenceNumber"], 1);
    assert_eq!(responses[4]["SequenceNumber"], 2);
}

#[test]
fn invalid_receipt_handles_are_those_never_issued() {
    let handles = [
        "r-01", "R-1", "r-1 ", " r-1", "r-2", "r-0", "m-1", "", "r-+1", "r-1.0",
    ];
    let mut commands = vec![create("q", json!({})), send("q", "a"), receive("q")];
    commands.extend(handles.iter().map(|h| delete("q", h)));
    commands.extend(handles.iter().map(|h| change_visibility("q", h, 5)));
    let responses = run_values(&commands);
    for response in &responses[3..] {
        assert_eq!(error_code(response), "ReceiptHandleInvalid");
    }
}

#[test]
fn handles_that_were_issued_but_are_not_usable_are_expired() {
    let responses = run_values(&[
        create("a", json!({})),
        create("b", json!({})),
        send("a", "1"),
        receive("a"),
        // Issued for a message of another queue.
        delete("b", "r-1"),
        tick(30),
        // The visibility timeout has ended.
        delete("a", "r-1"),
        receive("a"),
        // Delivered again since.
        delete("a", "r-1"),
        change_visibility("a", "r-1", 1),
        delete("a", "r-2"),
        // Gone.
        delete("a", "r-2"),
        change_visibility("a", "r-2", 1),
    ]);
    for index in [4, 6, 8, 9, 11, 12] {
        assert_eq!(
            error_code(&responses[index]),
            "ReceiptHandleExpired",
            "line {index}"
        );
    }
    assert_eq!(responses[10], json!({"ok": true}));
}

#[test]
fn handles_do_not_survive_a_purge_or_a_recreated_queue() {
    let responses = run_values(&[
        create("q", json!({})),
        send("q", "a"),
        receive("q"),
        json!({"Command": "PurgeQueue", "QueueName": "q"}),
        delete("q", "r-1"),
        send("q", "b"),
        receive("q"),
        json!({"Command": "DeleteQueue", "QueueName": "q"}),
        create("q", json!({})),
        delete("q", "r-2"),
    ]);
    assert_eq!(error_code(&responses[4]), "ReceiptHandleExpired");
    assert_eq!(error_code(&responses[9]), "ReceiptHandleExpired");
}

#[test]
fn delete_message_removes_the_message_for_good() {
    let responses = run_values(&[
        create("q", json!({})),
        send("q", "a"),
        receive("q"),
        delete("q", "r-1"),
        counts("q"),
        tick(1000),
        receive("q"),
    ]);
    assert_eq!(responses[3], json!({"ok": true}));
    assert_eq!(state_counts(&responses[4]), [0, 0, 0]);
    assert_eq!(bodies(&responses[6]), Vec::<&str>::new());
}

#[test]
fn change_message_visibility_restarts_the_timeout_from_now() {
    let responses = run_values(&[
        create("q", json!({"VisibilityTimeout": 10})),
        send("q", "a"),
        receive("q"),
        tick(8),
        change_visibility("q", "r-1", 10),
        tick(9),
        counts("q"),
        tick(1),
        counts("q"),
    ]);
    assert_eq!(responses[4], json!({"ok": true}));
    assert_eq!(state_counts(&responses[6]), [0, 1, 0]);
    assert_eq!(state_counts(&responses[8]), [1, 0, 0]);
}

#[test]
fn change_message_visibility_can_shorten_and_keeps_the_receive_count() {
    let responses = run_values(&[
        create("q", json!({})),
        send("q", "a"),
        receive("q"),
        change_visibility("q", "r-1", 0),
        counts("q"),
        receive("q"),
    ]);
    assert_eq!(state_counts(&responses[4]), [1, 0, 0]);
    assert_eq!(responses[5]["Messages"][0]["ReceiveCount"], 2);
}

#[test]
fn change_message_visibility_checks_its_timeout_range() {
    let responses = run_values(&[
        create("q", json!({})),
        send("q", "a"),
        receive("q"),
        change_visibility("q", "r-1", 43201),
        change_visibility("q", "r-1", 43200),
    ]);
    assert_eq!(error_code(&responses[3]), "InvalidParameterValue");
    assert_eq!(responses[4], json!({"ok": true}));
}

#[test]
fn a_message_changing_hands_is_only_deletable_with_its_latest_handle() {
    let responses = run_values(&[
        create("q", json!({"VisibilityTimeout": 5})),
        send("q", "a"),
        receive("q"),
        tick(5),
        receive("q"),
        delete("q", "r-1"),
        delete("q", "r-2"),
    ]);
    assert_eq!(error_code(&responses[5]), "ReceiptHandleExpired");
    assert_eq!(responses[6], json!({"ok": true}));
}
