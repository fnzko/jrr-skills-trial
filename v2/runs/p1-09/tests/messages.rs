mod common;

use common::{error_code, ok, outcomes, session};
use serde_json::{Value, json};

fn create(name: &str) -> Value {
    json!({"Command": "CreateQueue", "QueueName": name})
}

fn create_with(name: &str, attributes: Value) -> Value {
    json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attributes})
}

fn create_fifo(name: &str) -> Value {
    create_with(name, json!({"Fifo": true}))
}

fn send(queue: &str, body: &str) -> Value {
    json!({"Command": "SendMessage", "QueueName": queue, "Body": body})
}

fn send_fifo(queue: &str, body: &str, group: &str, dedup: &str) -> Value {
    json!({"Command": "SendMessage", "QueueName": queue, "Body": body,
           "GroupId": group, "DeduplicationId": dedup})
}

fn receive(queue: &str) -> Value {
    json!({"Command": "ReceiveMessage", "QueueName": queue})
}

fn receive_max(queue: &str, max: u32) -> Value {
    json!({"Command": "ReceiveMessage", "QueueName": queue, "MaxMessages": max})
}

fn tick(seconds: u64) -> Value {
    json!({"Command": "Tick", "Seconds": seconds})
}

fn delete(queue: &str, handle: &str) -> Value {
    json!({"Command": "DeleteMessage", "QueueName": queue, "ReceiptHandle": handle})
}

fn change(queue: &str, handle: &str, timeout: u64) -> Value {
    json!({"Command": "ChangeMessageVisibility", "QueueName": queue,
           "ReceiptHandle": handle, "VisibilityTimeout": timeout})
}

fn counts(queue: &str) -> Value {
    json!({"Command": "GetQueueAttributes", "QueueName": queue})
}

/// The bodies of a `ReceiveMessage` response, in order.
fn bodies(response: &Value) -> Vec<&str> {
    response["Messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|message| message["Body"].as_str().unwrap())
        .collect()
}

fn count_of<'a>(response: &'a Value, state: &str) -> &'a Value {
    &response["Counts"][state]
}

#[test]
fn message_ids_count_accepted_messages_over_all_queues() {
    let responses = session(&[
        create("a"),
        create("b"),
        send("a", "x"),
        send("b", "y"),
        send("a", ""),
        send("a", "z"),
    ]);
    assert_eq!(responses[2], json!({"ok": true, "MessageId": "m-1"}));
    assert_eq!(responses[3], json!({"ok": true, "MessageId": "m-2"}));
    assert_eq!(error_code(&responses[4]), "InvalidParameterValue");
    assert_eq!(responses[5], json!({"ok": true, "MessageId": "m-3"}));
}

#[test]
fn bodies_are_limited_by_their_utf8_length() {
    let two_byte = |chars: usize| "é".repeat(chars);
    let responses = session(&[
        create("q"),
        send("q", &"a".repeat(1024)),
        send("q", &"a".repeat(1025)),
        send("q", &two_byte(512)),
        send("q", &two_byte(513)),
        send("q", ""),
        json!({"Command": "SendMessage", "QueueName": "q", "Body": 5}),
    ]);
    assert_eq!(
        outcomes(&responses[1..]),
        [
            "ok",
            "InvalidParameterValue",
            "ok",
            "InvalidParameterValue",
            "InvalidParameterValue",
            "InvalidParameterValue"
        ]
    );
}

#[test]
fn send_checks_parameters_in_the_documented_order() {
    let responses = session(&[
        create("q"),
        json!({"Command": "SendMessage", "Body": "x"}),
        json!({"Command": "SendMessage", "QueueName": "q", "DelaySeconds": 901}),
        json!({"Command": "SendMessage", "QueueName": "q", "Body": "x", "DelaySeconds": 901}),
        json!({"Command": "SendMessage", "QueueName": "q", "Body": "x", "DelaySeconds": 900}),
        json!({"Command": "SendMessage", "QueueName": "q", "Body": "x", "GroupId": "bad id"}),
        json!({"Command": "SendMessage", "QueueName": "q", "Body": "x", "GroupId": "g"}),
        json!({"Command": "SendMessage", "QueueName": "q", "Body": "x", "DeduplicationId": "d"}),
        json!({"Command": "SendMessage", "QueueName": "nope", "Body": "x", "GroupId": "g"}),
        json!({"Command": "SendMessage", "QueueName": "q", "Body": "x", "GroupId": "a".repeat(129)}),
        json!({"Command": "SendMessage", "QueueName": "q", "Body": "x", "GroupId": null}),
    ]);
    assert_eq!(
        outcomes(&responses[1..]),
        [
            "MissingParameter",
            "MissingParameter",
            "InvalidParameterValue",
            "ok",
            "InvalidParameterValue",
            "UnsupportedOperation",
            "UnsupportedOperation",
            "QueueDoesNotExist",
            "InvalidParameterValue",
            "ok",
        ]
    );
}

#[test]
fn a_delivered_message_carries_its_bookkeeping() {
    let responses = session(&[
        create("q"),
        tick(3),
        send("q", "hello"),
        tick(5),
        receive("q"),
        receive("q"),
        counts("q"),
    ]);
    assert_eq!(
        responses[4],
        json!({"ok": true, "Messages": [{
            "MessageId": "m-1",
            "ReceiptHandle": "r-1",
            "Body": "hello",
            "ReceiveCount": 1,
            "SentAt": 3,
            "FirstReceivedAt": 8,
        }]})
    );
    assert_eq!(responses[5], json!({"ok": true, "Messages": []}));
    assert_eq!(count_of(&responses[6], "InFlight"), 1);
    assert_eq!(count_of(&responses[6], "Visible"), 0);
}

#[test]
fn receive_delivers_in_arrival_order_up_to_the_maximum() {
    let responses = session(&[
        create("q"),
        send("q", "one"),
        send("q", "two"),
        send("q", "three"),
        receive_max("q", 2),
        receive("q"),
        receive("q"),
        receive_max("q", 0),
        receive_max("q", 11),
        json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": "2"}),
    ]);
    assert_eq!(bodies(&responses[4]), ["one", "two"]);
    assert_eq!(bodies(&responses[5]), ["three"]);
    assert!(bodies(&responses[6]).is_empty());
    assert_eq!(
        outcomes(&responses[7..]),
        [
            "InvalidParameterValue",
            "InvalidParameterValue",
            "InvalidParameterValue"
        ]
    );
}

#[test]
fn receipt_handles_count_deliveries_over_all_queues() {
    let responses = session(&[
        create("a"),
        create("b"),
        send("a", "x"),
        send("b", "y"),
        receive("b"),
        receive("a"),
        tick(30),
        receive("a"),
    ]);
    assert_eq!(responses[4]["Messages"][0]["ReceiptHandle"], "r-1");
    assert_eq!(responses[5]["Messages"][0]["ReceiptHandle"], "r-2");
    assert_eq!(responses[7]["Messages"][0]["ReceiptHandle"], "r-3");
}

#[test]
fn a_message_returns_when_its_visibility_timeout_ends_and_keeps_its_place() {
    let responses = session(&[
        create_with("q", json!({"VisibilityTimeout": 10})),
        send("q", "first"),
        send("q", "second"),
        receive("q"),
        tick(9),
        receive_max("q", 5),
        tick(1),
        receive_max("q", 5),
    ]);
    assert_eq!(bodies(&responses[5]), ["second"]);
    let again = &responses[7]["Messages"];
    assert_eq!(again[0]["Body"], "first");
    assert_eq!(again[0]["ReceiveCount"], 2);
    assert_eq!(again[0]["FirstReceivedAt"], 0);
    assert_eq!(again[0]["ReceiptHandle"], "r-3");
}

#[test]
fn a_visibility_timeout_in_the_request_beats_the_queue_attribute() {
    let responses = session(&[
        create("q"),
        send("q", "x"),
        json!({"Command": "ReceiveMessage", "QueueName": "q", "VisibilityTimeout": 2}),
        tick(1),
        receive("q"),
        tick(1),
        receive("q"),
        json!({"Command": "ReceiveMessage", "QueueName": "q", "VisibilityTimeout": 43201}),
    ]);
    assert_eq!(bodies(&responses[2]), ["x"]);
    assert!(bodies(&responses[4]).is_empty());
    assert_eq!(bodies(&responses[6]), ["x"]);
    assert_eq!(error_code(&responses[7]), "InvalidParameterValue");
}

#[test]
fn a_zero_visibility_timeout_makes_the_handle_unusable() {
    let responses = session(&[
        create("q"),
        send("q", "x"),
        json!({"Command": "ReceiveMessage", "QueueName": "q", "VisibilityTimeout": 0}),
        delete("q", "r-1"),
        receive("q"),
    ]);
    assert_eq!(error_code(&responses[3]), "ReceiptHandleExpired");
    assert_eq!(responses[4]["Messages"][0]["ReceiveCount"], 2);
}

#[test]
fn delayed_messages_become_visible_when_the_delay_ends() {
    let responses = session(&[
        create_with("q", json!({"DelaySeconds": 5})),
        send("q", "queue-default"),
        json!({"Command": "SendMessage", "QueueName": "q", "Body": "now", "DelaySeconds": 0}),
        json!({"Command": "SendMessage", "QueueName": "q", "Body": "late", "DelaySeconds": 10}),
        counts("q"),
        receive_max("q", 10),
        tick(5),
        counts("q"),
        receive_max("q", 10),
        tick(5),
        receive_max("q", 10),
    ]);
    assert_eq!(count_of(&responses[4], "Delayed"), 2);
    assert_eq!(count_of(&responses[4], "Visible"), 1);
    assert_eq!(bodies(&responses[5]), ["now"]);
    assert_eq!(count_of(&responses[7], "Delayed"), 1);
    assert_eq!(bodies(&responses[8]), ["queue-default"]);
    assert_eq!(bodies(&responses[10]), ["late"]);
}

#[test]
fn a_new_delay_applies_to_later_sends_only() {
    let responses = session(&[
        create_with("q", json!({"DelaySeconds": 5})),
        send("q", "old"),
        json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"DelaySeconds": 0}}),
        send("q", "new"),
        receive_max("q", 10),
        tick(5),
        receive_max("q", 10),
    ]);
    assert_eq!(bodies(&responses[4]), ["new"]);
    assert_eq!(bodies(&responses[6]), ["old"]);
}

#[test]
fn messages_are_removed_when_their_retention_runs_out() {
    let responses = session(&[
        create_with(
            "q",
            json!({"RetentionSeconds": 60, "VisibilityTimeout": 1000}),
        ),
        send("q", "in flight"),
        receive("q"),
        tick(30),
        send("q", "visible"),
        tick(29),
        counts("q"),
        tick(1),
        counts("q"),
        delete("q", "r-1"),
        tick(30),
        counts("q"),
    ]);
    assert_eq!(count_of(&responses[6], "InFlight"), 1);
    assert_eq!(count_of(&responses[6], "Visible"), 1);
    assert_eq!(count_of(&responses[8], "InFlight"), 0);
    assert_eq!(count_of(&responses[8], "Visible"), 1);
    assert_eq!(error_code(&responses[9]), "ReceiptHandleExpired");
    assert_eq!(count_of(&responses[11], "Visible"), 0);
}

#[test]
fn a_lower_retention_removes_messages_at_once_and_a_higher_one_restores_nothing() {
    let responses = session(&[
        create("lowered"),
        create_with("raised", json!({"RetentionSeconds": 60})),
        send("lowered", "x"),
        send("raised", "y"),
        tick(100),
        json!({"Command": "SetQueueAttributes", "QueueName": "lowered",
               "Attributes": {"RetentionSeconds": 100}}),
        json!({"Command": "SetQueueAttributes", "QueueName": "raised",
               "Attributes": {"RetentionSeconds": 1000}}),
        counts("lowered"),
        counts("raised"),
    ]);
    assert_eq!(count_of(&responses[7], "Visible"), 0);
    assert_eq!(count_of(&responses[8], "Visible"), 0);
}

#[test]
fn a_standard_queue_has_no_sequence_numbers() {
    let responses = session(&[create("q"), send("q", "x"), tick(0), receive("q")]);
    assert!(responses[1].get("SequenceNumber").is_none());
    assert!(responses[3]["Messages"][0].get("SequenceNumber").is_none());
    assert!(responses[3]["Messages"][0].get("GroupId").is_none());
}

#[test]
fn fifo_sends_need_a_group_and_a_deduplication_id() {
    let responses = session(&[
        create_fifo("f.fifo"),
        send("f.fifo", "x"),
        json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "x", "GroupId": "g"}),
        json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "x",
               "DeduplicationId": "d"}),
        json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "x", "GroupId": "g",
               "DeduplicationId": "d", "DelaySeconds": 0}),
        send_fifo("f.fifo", "x", "g", "d"),
        send_fifo("f.fifo", "y", "g", "e"),
    ]);
    assert_eq!(
        outcomes(&responses[1..]),
        [
            "MissingParameter",
            "MissingParameter",
            "MissingParameter",
            "UnsupportedOperation",
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
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2})
    );
}

#[test]
fn a_fifo_receive_takes_one_message_per_group_and_keeps_groups_in_order() {
    let responses = session(&[
        create_fifo("f.fifo"),
        send_fifo("f.fifo", "a1", "a", "1"),
        send_fifo("f.fifo", "a2", "a", "2"),
        send_fifo("f.fifo", "b1", "b", "3"),
        receive_max("f.fifo", 10),
        receive_max("f.fifo", 10),
        delete("f.fifo", "r-1"),
        receive_max("f.fifo", 10),
    ]);
    assert_eq!(
        responses[4]["Messages"],
        json!([
            {"MessageId": "m-1", "ReceiptHandle": "r-1", "Body": "a1", "ReceiveCount": 1,
             "SentAt": 0, "FirstReceivedAt": 0, "GroupId": "a", "SequenceNumber": 1},
            {"MessageId": "m-3", "ReceiptHandle": "r-2", "Body": "b1", "ReceiveCount": 1,
             "SentAt": 0, "FirstReceivedAt": 0, "GroupId": "b", "SequenceNumber": 3},
        ])
    );
    assert!(bodies(&responses[5]).is_empty());
    assert_eq!(bodies(&responses[7]), ["a2"]);
}

#[test]
fn a_delayed_message_blocks_later_messages_of_its_group() {
    let responses = session(&[
        create_with("f.fifo", json!({"Fifo": true, "DelaySeconds": 10})),
        send_fifo("f.fifo", "first", "g", "1"),
        json!({"Command": "SetQueueAttributes", "QueueName": "f.fifo",
               "Attributes": {"DelaySeconds": 0}}),
        send_fifo("f.fifo", "second", "g", "2"),
        send_fifo("f.fifo", "other", "h", "3"),
        receive_max("f.fifo", 10),
        tick(10),
        receive_max("f.fifo", 10),
    ]);
    assert_eq!(bodies(&responses[5]), ["other"]);
    assert_eq!(bodies(&responses[7]), ["first"]);
}

#[test]
fn a_repeated_deduplication_id_returns_the_original_message() {
    let responses = session(&[
        create_fifo("f.fifo"),
        create_fifo("g.fifo"),
        send_fifo("f.fifo", "x", "a", "d"),
        send_fifo("f.fifo", "other body", "b", "d"),
        send_fifo("g.fifo", "x", "a", "d"),
        send_fifo("f.fifo", "y", "a", "e"),
    ]);
    assert_eq!(
        responses[2],
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
    assert_eq!(responses[3], responses[2]);
    assert_eq!(
        responses[4],
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 1})
    );
    assert_eq!(
        responses[5],
        json!({"ok": true, "MessageId": "m-3", "SequenceNumber": 2})
    );
}

#[test]
fn the_deduplication_window_lasts_300_seconds_and_repeats_do_not_extend_it() {
    let responses = session(&[
        create_fifo("f.fifo"),
        send_fifo("f.fifo", "x", "g", "d"),
        tick(200),
        send_fifo("f.fifo", "x", "g", "d"),
        tick(99),
        send_fifo("f.fifo", "x", "g", "d"),
        tick(1),
        send_fifo("f.fifo", "x", "g", "d"),
        tick(299),
        send_fifo("f.fifo", "x", "g", "d"),
        tick(1),
        send_fifo("f.fifo", "x", "g", "d"),
    ]);
    let ids: Vec<&str> = [1, 3, 5, 7, 9, 11]
        .iter()
        .map(|&i| responses[i]["MessageId"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["m-1", "m-1", "m-1", "m-2", "m-2", "m-3"]);
}

#[test]
fn deduplication_ids_outlive_their_messages_and_survive_a_purge() {
    let responses = session(&[
        create_fifo("f.fifo"),
        send_fifo("f.fifo", "x", "g", "d"),
        send_fifo("f.fifo", "y", "g", "e"),
        receive("f.fifo"),
        delete("f.fifo", "r-1"),
        send_fifo("f.fifo", "x", "g", "d"),
        json!({"Command": "PurgeQueue", "QueueName": "f.fifo"}),
        send_fifo("f.fifo", "y", "g", "e"),
        send_fifo("f.fifo", "z", "g", "f"),
    ]);
    assert_eq!(
        responses[5],
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
    assert_eq!(
        responses[7],
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2})
    );
    assert_eq!(
        responses[8],
        json!({"ok": true, "MessageId": "m-3", "SequenceNumber": 3})
    );
}

#[test]
fn content_based_deduplication_uses_the_body_unless_an_id_is_given() {
    let responses = session(&[
        create_with(
            "f.fifo",
            json!({"Fifo": true, "ContentBasedDeduplication": true}),
        ),
        json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "same", "GroupId": "g"}),
        json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "same", "GroupId": "h"}),
        json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "same", "GroupId": "g",
               "DeduplicationId": "explicit"}),
        json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "other", "GroupId": "g"}),
    ]);
    let ids: Vec<&str> = responses[1..]
        .iter()
        .map(|r| r["MessageId"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["m-1", "m-1", "m-2", "m-3"]);
}

#[test]
fn a_queue_holds_at_most_100000_messages() {
    let mut commands = vec![create("q")];
    commands.extend((0..100_000).map(|_| send("q", "x")));
    commands.push(send("q", "one too many"));
    commands.push(json!({"Command": "PurgeQueue", "QueueName": "q"}));
    commands.push(send("q", "fits again"));
    let responses = session(&commands);
    assert!(responses[1..=100_000].iter().all(|r| r["ok"] == true));
    assert_eq!(responses[100_000]["MessageId"], "m-100000");
    assert_eq!(error_code(&responses[100_001]), "LimitExceeded");
    assert_eq!(responses[100_003]["MessageId"], "m-100001");
}

#[test]
fn delete_message_removes_the_message_once() {
    let responses = session(&[
        create("q"),
        send("q", "x"),
        receive("q"),
        delete("q", "r-1"),
        delete("q", "r-1"),
        counts("q"),
    ]);
    assert_eq!(responses[3], ok());
    assert_eq!(error_code(&responses[4]), "ReceiptHandleExpired");
    assert_eq!(count_of(&responses[5], "InFlight"), 0);
}

#[test]
fn only_exact_spellings_of_issued_handles_are_known() {
    let mut commands = vec![create("q"), send("q", "x"), receive("q")];
    let never_issued = [
        "r-01",
        "R-1",
        "r-1 ",
        " r-1",
        "r-2",
        "r-0",
        "r-",
        "r-+1",
        "r--1",
        "1",
        "",
        "r-1.0",
        "r-99999999999999999999999",
    ];
    commands.extend(never_issued.iter().map(|handle| delete("q", handle)));
    let responses = session(&commands);
    for (handle, response) in never_issued.iter().zip(&responses[3..]) {
        assert_eq!(
            error_code(response),
            "ReceiptHandleInvalid",
            "handle {handle:?}"
        );
    }
}

#[test]
fn a_handle_stops_working_when_the_message_is_delivered_again_or_times_out() {
    let responses = session(&[
        create_with("q", json!({"VisibilityTimeout": 5})),
        send("q", "x"),
        receive("q"),
        tick(5),
        delete("q", "r-1"),
        receive("q"),
        delete("q", "r-1"),
        change("q", "r-1", 10),
        delete("q", "r-2"),
    ]);
    assert_eq!(error_code(&responses[4]), "ReceiptHandleExpired");
    assert_eq!(error_code(&responses[6]), "ReceiptHandleExpired");
    assert_eq!(error_code(&responses[7]), "ReceiptHandleExpired");
    assert_eq!(responses[8], ok());
}

#[test]
fn a_handle_only_works_in_the_queue_that_issued_it() {
    let responses = session(&[
        create("a"),
        create("b"),
        send("a", "x"),
        receive("a"),
        delete("b", "r-1"),
        json!({"Command": "PurgeQueue", "QueueName": "a"}),
        delete("a", "r-1"),
        send("a", "y"),
        receive("a"),
        json!({"Command": "DeleteQueue", "QueueName": "a"}),
        create("a"),
        delete("a", "r-2"),
    ]);
    assert_eq!(error_code(&responses[4]), "ReceiptHandleExpired");
    assert_eq!(error_code(&responses[6]), "ReceiptHandleExpired");
    assert_eq!(error_code(&responses[11]), "ReceiptHandleExpired");
}

#[test]
fn change_message_visibility_replaces_the_end_of_the_timeout() {
    let responses = session(&[
        create("q"),
        send("q", "x"),
        receive("q"),
        tick(10),
        change("q", "r-1", 20),
        tick(19),
        receive("q"),
        tick(1),
        receive("q"),
    ]);
    assert_eq!(responses[4], ok());
    assert!(bodies(&responses[6]).is_empty());
    assert_eq!(responses[8]["Messages"][0]["ReceiveCount"], 2);
}

#[test]
fn change_message_visibility_to_zero_makes_the_message_visible_at_once() {
    let responses = session(&[
        create("q"),
        send("q", "x"),
        receive("q"),
        change("q", "r-1", 0),
        counts("q"),
        change("q", "r-1", 5),
        receive("q"),
    ]);
    assert_eq!(responses[3], ok());
    assert_eq!(count_of(&responses[4], "Visible"), 1);
    assert_eq!(error_code(&responses[5]), "ReceiptHandleExpired");
    assert_eq!(responses[6]["Messages"][0]["ReceiveCount"], 2);
}

#[test]
fn change_message_visibility_checks_its_parameters_in_order() {
    let responses = session(&[
        create("q"),
        json!({"Command": "ChangeMessageVisibility", "QueueName": "q", "ReceiptHandle": 5}),
        json!({"Command": "ChangeMessageVisibility", "QueueName": "q", "ReceiptHandle": "r-1",
               "VisibilityTimeout": 43201}),
        change("nope", "bogus", 5),
        change("q", "bogus", 5),
        json!({"Command": "DeleteMessage", "QueueName": "q"}),
    ]);
    assert_eq!(
        outcomes(&responses[1..]),
        [
            "MissingParameter",
            "InvalidParameterValue",
            "QueueDoesNotExist",
            "ReceiptHandleInvalid",
            "MissingParameter",
        ]
    );
}

#[test]
fn a_message_moves_between_states_and_the_counts_follow() {
    let responses = session(&[
        create("q"),
        send("q", "a"),
        json!({"Command": "SendMessage", "QueueName": "q", "Body": "b", "DelaySeconds": 3}),
        send("q", "c"),
        receive("q"),
        counts("q"),
    ]);
    assert_eq!(
        responses[5]["Counts"],
        json!({"Visible": 1, "InFlight": 1, "Delayed": 1})
    );
}
