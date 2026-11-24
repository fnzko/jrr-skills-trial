use std::io::{Cursor, Write};
use std::process::{Command, Stdio};

use serde_json::{Value, json};

/// Runs `input` through miniq and returns one parsed response per output line.
fn session(input: &str) -> Vec<Value> {
    let mut output = Vec::new();
    miniq::run(Cursor::new(input.as_bytes()), &mut output).unwrap();
    String::from_utf8(output)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn lines(commands: &[Value]) -> String {
    commands
        .iter()
        .map(|command| format!("{command}\n"))
        .collect()
}

fn ok() -> Value {
    json!({"ok": true})
}

fn failure(code: &str) -> Value {
    json!({"ok": false, "error": {"code": code, "message": "*"}})
}

/// Replaces every error message by `*` so tests compare codes only.
fn normalized(responses: Vec<Value>) -> Vec<Value> {
    responses
        .into_iter()
        .map(|mut response| {
            if let Some(message) = response.pointer_mut("/error/message") {
                *message = json!("*");
            }
            response
        })
        .collect()
}

fn run(commands: &[Value]) -> Vec<Value> {
    normalized(session(&lines(commands)))
}

fn create(name: &str) -> Value {
    json!({"Command": "CreateQueue", "QueueName": name})
}

fn create_with(name: &str, attributes: Value) -> Value {
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

fn message(id: &str, receipt: &str, body: &str, count: u32, sent_at: u32, first: u32) -> Value {
    json!({"MessageId": id, "ReceiptHandle": receipt, "Body": body,
           "ReceiveCount": count, "SentAt": sent_at, "FirstReceivedAt": first})
}

#[test]
fn every_line_gets_one_response_in_order() {
    let responses = normalized(session(
        "\n{\"Command\":\"Tick\",\"Seconds\":5}\nnot json\n{\"Command\":\"Tick\",\"Seconds\":2}",
    ));
    assert_eq!(
        responses,
        vec![
            failure("InvalidRequest"),
            json!({"ok": true, "Now": 5}),
            failure("InvalidRequest"),
            json!({"ok": true, "Now": 7}),
        ]
    );
}

#[test]
fn input_that_is_not_utf8_is_answered_and_skipped() {
    let mut input = b"\xff\xfe\n".to_vec();
    input.extend_from_slice(b"{\"Command\":\"Tick\",\"Seconds\":1}\n");
    let mut output = Vec::new();
    miniq::run(Cursor::new(input), &mut output).unwrap();
    let responses: Vec<Value> = String::from_utf8(output)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        normalized(responses),
        vec![failure("InvalidRequest"), json!({"ok": true, "Now": 1})]
    );
}

#[test]
fn binary_answers_and_exits_zero_at_end_of_input() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_miniq"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"{\"Command\":\"Tick\",\"Seconds\":3}")
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    assert_eq!(output.stdout.last(), Some(&b'\n'));
    let response: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(response, json!({"ok": true, "Now": 3}));
}

#[test]
fn error_codes_follow_the_documented_precedence() {
    let responses = run(&[
        json!({"Command": "SendMessage", "QueueName": 7}),
        json!({"Command": "SendMessage", "QueueName": "nope", "Body": ""}),
        json!({"Command": "SendMessage", "QueueName": "nope", "Body": "x"}),
        json!({"Command": "CreateQueue", "QueueName": "bad name", "Attributes": 1}),
        json!({"Command": "CreateQueue", "QueueName": "a.fifo"}),
        json!({"Command": "CreateQueue", "QueueName": "a", "Attributes": {"Fifo": true}}),
        json!({"Command": "DeleteMessage", "QueueName": "not a name!", "ReceiptHandle": "x"}),
    ]);
    assert_eq!(
        responses,
        vec![
            failure("MissingParameter"),
            failure("InvalidParameterValue"),
            failure("QueueDoesNotExist"),
            failure("QueueNameInvalid"),
            failure("QueueNameInvalid"),
            failure("QueueNameInvalid"),
            failure("QueueDoesNotExist"),
        ]
    );
}

#[test]
fn create_queue_is_idempotent_only_for_equal_attributes() {
    let responses = run(&[
        create("jobs"),
        create_with(
            "jobs",
            json!({"VisibilityTimeout": 30, "Fifo": false, "DeadLetterQueue": null}),
        ),
        create_with("jobs", json!({"VisibilityTimeout": 31})),
        create_with("jobs", json!({"DelaySeconds": 0})),
    ]);
    assert_eq!(
        responses,
        vec![ok(), ok(), failure("QueueAlreadyExists"), ok()]
    );
}

#[test]
fn create_queue_rules_run_in_the_documented_order() {
    let responses = run(&[
        create_with("a", json!({"ContentBasedDeduplication": true})),
        create_with("a", json!({"MaxReceiveCount": 2})),
        create_with(
            "a",
            json!({"DeadLetterQueue": "missing", "MaxReceiveCount": 2}),
        ),
        create("std"),
        create_with(
            "b.fifo",
            json!({"Fifo": true, "DeadLetterQueue": "std", "MaxReceiveCount": 2}),
        ),
        create_with(
            "std",
            json!({"DeadLetterQueue": "missing", "MaxReceiveCount": 2}),
        ),
    ]);
    assert_eq!(
        responses,
        vec![
            failure("InvalidParameterValue"),
            failure("InvalidParameterValue"),
            failure("QueueDoesNotExist"),
            ok(),
            failure("InvalidParameterValue"),
            failure("QueueAlreadyExists"),
        ]
    );
}

#[test]
fn queue_count_is_limited_to_1000() {
    let mut input = String::new();
    for n in 0..1001 {
        input.push_str(&format!(
            "{{\"Command\":\"CreateQueue\",\"QueueName\":\"q{n}\"}}\n"
        ));
    }
    let responses = normalized(session(&input));
    assert_eq!(responses.len(), 1001);
    assert_eq!(responses[999], ok());
    assert_eq!(responses[1000], failure("LimitExceeded"));
}

#[test]
fn list_queues_sorts_by_byte_value_and_filters_by_prefix() {
    let responses = run(&[
        create("b"),
        create("a_1"),
        create("a-1"),
        create("A1"),
        create_with("a1.fifo", json!({"Fifo": true})),
        json!({"Command": "ListQueues"}),
        json!({"Command": "ListQueues", "Prefix": "a"}),
        json!({"Command": "ListQueues", "Prefix": "zzz"}),
    ]);
    assert_eq!(
        responses[5..],
        [
            json!({"ok": true, "QueueNames": ["A1", "a-1", "a1.fifo", "a_1", "b"]}),
            json!({"ok": true, "QueueNames": ["a-1", "a1.fifo", "a_1"]}),
            json!({"ok": true, "QueueNames": []}),
        ]
    );
}

#[test]
fn get_queue_attributes_reports_defaults_and_counts() {
    let responses = run(&[
        create("q"),
        send("q", "one"),
        json!({"Command": "SendMessage", "QueueName": "q", "Body": "two", "DelaySeconds": 10}),
        receive("q"),
        counts("q"),
    ]);
    assert_eq!(
        responses[4],
        json!({"ok": true,
               "Attributes": {"VisibilityTimeout": 30, "DelaySeconds": 0, "RetentionSeconds": 345600,
                              "Fifo": false, "ContentBasedDeduplication": false},
               "Counts": {"Visible": 0, "InFlight": 1, "Delayed": 1}})
    );
}

#[test]
fn messages_get_global_ids_and_handles() {
    let responses = run(&[
        create("a"),
        create("b"),
        send("a", "x"),
        send("b", "y"),
        json!({"Command": "SendMessage", "QueueName": "a", "Body": ""}),
        send("a", "z"),
        receive("b"),
        receive("a"),
    ]);
    assert_eq!(responses[2], json!({"ok": true, "MessageId": "m-1"}));
    assert_eq!(responses[3], json!({"ok": true, "MessageId": "m-2"}));
    assert_eq!(responses[5], json!({"ok": true, "MessageId": "m-3"}));
    assert_eq!(
        responses[6],
        json!({"ok": true, "Messages": [message("m-2", "r-1", "y", 1, 0, 0)]})
    );
    assert_eq!(
        responses[7],
        json!({"ok": true, "Messages": [message("m-1", "r-2", "x", 1, 0, 0)]})
    );
}

#[test]
fn delayed_message_becomes_visible_at_send_time_plus_delay() {
    let responses = run(&[
        create_with("q", json!({"DelaySeconds": 5})),
        send("q", "x"),
        tick(4),
        receive("q"),
        tick(1),
        receive("q"),
    ]);
    assert_eq!(responses[3], json!({"ok": true, "Messages": []}));
    assert_eq!(
        responses[5],
        json!({"ok": true, "Messages": [message("m-1", "r-1", "x", 1, 0, 5)]})
    );
}

#[test]
fn visibility_timeout_hides_a_message_and_it_keeps_its_place() {
    let responses = run(&[
        create("q"),
        send("q", "first"),
        send("q", "second"),
        json!({"Command": "ReceiveMessage", "QueueName": "q", "VisibilityTimeout": 10}),
        json!({"Command": "ReceiveMessage", "QueueName": "q", "VisibilityTimeout": 1}),
        tick(10),
        json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 2}),
    ]);
    assert_eq!(
        responses[6],
        json!({"ok": true, "Messages": [
            message("m-1", "r-3", "first", 2, 0, 0),
            message("m-2", "r-4", "second", 2, 0, 0),
        ]})
    );
}

#[test]
fn receive_returns_up_to_max_messages_in_arrival_order() {
    let responses = run(&[
        create("q"),
        send("q", "a"),
        send("q", "b"),
        send("q", "c"),
        json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 2}),
        json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 11}),
    ]);
    assert_eq!(
        responses[4],
        json!({"ok": true, "Messages": [
            message("m-1", "r-1", "a", 1, 0, 0),
            message("m-2", "r-2", "b", 1, 0, 0),
        ]})
    );
    assert_eq!(responses[5], failure("InvalidParameterValue"));
}

#[test]
fn delete_and_change_visibility_check_handles() {
    let responses = run(&[
        create("q"),
        create("other"),
        send("q", "x"),
        receive("q"),
        json!({"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": "r-01"}),
        json!({"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": "r-2"}),
        json!({"Command": "DeleteMessage", "QueueName": "other", "ReceiptHandle": "r-1"}),
        json!({"Command": "ChangeMessageVisibility", "QueueName": "q", "ReceiptHandle": "r-1", "VisibilityTimeout": 0}),
        json!({"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": "r-1"}),
        receive("q"),
        json!({"Command": "ChangeMessageVisibility", "QueueName": "q", "ReceiptHandle": "r-2", "VisibilityTimeout": 20}),
        tick(19),
        counts("q"),
        tick(1),
        json!({"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": "r-2"}),
    ]);
    assert_eq!(responses[4], failure("ReceiptHandleInvalid"));
    assert_eq!(responses[5], failure("ReceiptHandleInvalid"));
    assert_eq!(responses[6], failure("ReceiptHandleExpired"));
    assert_eq!(responses[7], ok());
    assert_eq!(responses[8], failure("ReceiptHandleExpired"));
    assert_eq!(responses[10], ok());
    assert_eq!(
        responses[12]["Counts"],
        json!({"Visible": 0, "InFlight": 1, "Delayed": 0})
    );
    assert_eq!(responses[14], failure("ReceiptHandleExpired"));
}

#[test]
fn delete_message_removes_it() {
    let responses = run(&[
        create("q"),
        send("q", "x"),
        receive("q"),
        json!({"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": "r-1"}),
        counts("q"),
    ]);
    assert_eq!(responses[3], ok());
    assert_eq!(
        responses[4]["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn retention_removes_messages_and_lowering_it_applies_at_once() {
    let responses = run(&[
        create_with("q", json!({"RetentionSeconds": 100})),
        send("q", "a"),
        tick(60),
        send("q", "b"),
        json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"RetentionSeconds": 60}}),
        json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"RetentionSeconds": 1000}}),
        counts("q"),
        tick(999),
        counts("q"),
        tick(1),
        counts("q"),
    ]);
    let visible = |index: usize| responses[index]["Counts"]["Visible"].clone();
    assert_eq!(visible(6), json!(1));
    assert_eq!(visible(8), json!(1));
    assert_eq!(visible(10), json!(0));
}

#[test]
fn set_queue_attributes_rules() {
    let responses = run(&[
        create("q"),
        create("other"),
        create_with("f.fifo", json!({"Fifo": true})),
        json!({"Command": "SetQueueAttributes", "QueueName": "q"}),
        json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"Fifo": false}}),
        json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"ContentBasedDeduplication": true}}),
        json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"MaxReceiveCount": 3}}),
        json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"DeadLetterQueue": "gone", "MaxReceiveCount": 3}}),
        json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"DeadLetterQueue": "q", "MaxReceiveCount": 3}}),
        json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"DeadLetterQueue": "f.fifo", "MaxReceiveCount": 3}}),
        json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"DeadLetterQueue": "other", "MaxReceiveCount": 3, "DelaySeconds": 7}}),
        json!({"Command": "SetQueueAttributes", "QueueName": "nope", "Attributes": {}}),
        counts("q"),
    ]);
    assert_eq!(
        responses[3..12],
        [
            failure("MissingParameter"),
            failure("UnsupportedOperation"),
            failure("InvalidParameterValue"),
            failure("InvalidParameterValue"),
            failure("QueueDoesNotExist"),
            failure("InvalidParameterValue"),
            failure("InvalidParameterValue"),
            ok(),
            failure("QueueDoesNotExist"),
        ]
    );
    assert_eq!(
        responses[12]["Attributes"],
        json!({"VisibilityTimeout": 30, "DelaySeconds": 7, "RetentionSeconds": 345600,
               "Fifo": false, "ContentBasedDeduplication": false,
               "DeadLetterQueue": "other", "MaxReceiveCount": 3})
    );
}

#[test]
fn fifo_send_requires_group_and_deduplication() {
    let fifo = |extra: Value| {
        let mut command = json!({"Command": "SendMessage", "QueueName": "q.fifo", "Body": "x"});
        command
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        command
    };
    let responses = run(&[
        create_with("q.fifo", json!({"Fifo": true})),
        create("std"),
        fifo(json!({})),
        fifo(json!({"GroupId": "g"})),
        fifo(json!({"GroupId": "g", "DeduplicationId": "d", "DelaySeconds": 0})),
        fifo(json!({"GroupId": "bad group", "DeduplicationId": "d"})),
        fifo(json!({"GroupId": "g", "DeduplicationId": "d"})),
        json!({"Command": "SendMessage", "QueueName": "std", "Body": "x", "GroupId": "g"}),
        json!({"Command": "SendMessage", "QueueName": "std", "Body": "x", "DeduplicationId": "d"}),
    ]);
    assert_eq!(
        responses[2..],
        [
            failure("MissingParameter"),
            failure("MissingParameter"),
            failure("UnsupportedOperation"),
            failure("InvalidParameterValue"),
            json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1}),
            failure("UnsupportedOperation"),
            failure("UnsupportedOperation"),
        ]
    );
}

#[test]
fn deduplication_window_is_300_seconds_from_the_accepted_send() {
    let dup = |body: &str| json!({"Command": "SendMessage", "QueueName": "q.fifo", "Body": body, "GroupId": "g1"});
    let responses = run(&[
        create_with(
            "q.fifo",
            json!({"Fifo": true, "ContentBasedDeduplication": true}),
        ),
        dup("same"),
        tick(299),
        dup("same"),
        json!({"Command": "SendMessage", "QueueName": "q.fifo", "Body": "same", "GroupId": "g2"}),
        tick(1),
        dup("same"),
        dup("other"),
        tick(299),
        dup("same"),
        tick(1),
        dup("same"),
    ]);
    let sent = |index: usize| {
        (
            responses[index]["MessageId"].clone(),
            responses[index]["SequenceNumber"].clone(),
        )
    };
    assert_eq!(sent(1), (json!("m-1"), json!(1)));
    assert_eq!(sent(3), (json!("m-1"), json!(1)));
    assert_eq!(sent(4), (json!("m-1"), json!(1)));
    assert_eq!(sent(6), (json!("m-2"), json!(2)));
    assert_eq!(sent(7), (json!("m-3"), json!(3)));
    assert_eq!(sent(9), (json!("m-2"), json!(2)));
    assert_eq!(sent(11), (json!("m-4"), json!(4)));
}

#[test]
fn deduplication_outlives_the_message_and_purge() {
    let send_fifo = json!({"Command": "SendMessage", "QueueName": "q.fifo", "Body": "x", "GroupId": "g", "DeduplicationId": "d"});
    let responses = run(&[
        create_with("q.fifo", json!({"Fifo": true})),
        send_fifo.clone(),
        json!({"Command": "PurgeQueue", "QueueName": "q.fifo"}),
        send_fifo.clone(),
        json!({"Command": "SendMessage", "QueueName": "q.fifo", "Body": "y", "GroupId": "g", "DeduplicationId": "e"}),
        counts("q.fifo"),
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
        responses[5]["Counts"],
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn fifo_delivers_one_message_per_group_and_blocks_a_busy_group() {
    let fifo = |body: &str, group: &str| json!({"Command": "SendMessage", "QueueName": "q.fifo", "Body": body, "GroupId": group, "DeduplicationId": body});
    let responses = run(&[
        create_with("q.fifo", json!({"Fifo": true})),
        fifo("a1", "a"),
        fifo("a2", "a"),
        fifo("b1", "b"),
        json!({"Command": "ReceiveMessage", "QueueName": "q.fifo", "MaxMessages": 10}),
        json!({"Command": "ReceiveMessage", "QueueName": "q.fifo", "MaxMessages": 10}),
        json!({"Command": "DeleteMessage", "QueueName": "q.fifo", "ReceiptHandle": "r-1"}),
        json!({"Command": "ReceiveMessage", "QueueName": "q.fifo", "MaxMessages": 10}),
    ]);
    let delivered = |index: usize| -> Vec<(Value, Value, Value)> {
        responses[index]["Messages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| {
                (
                    m["Body"].clone(),
                    m["GroupId"].clone(),
                    m["SequenceNumber"].clone(),
                )
            })
            .collect()
    };
    assert_eq!(
        delivered(4),
        vec![
            (json!("a1"), json!("a"), json!(1)),
            (json!("b1"), json!("b"), json!(3))
        ]
    );
    assert_eq!(delivered(5), vec![]);
    assert_eq!(delivered(7), vec![(json!("a2"), json!("a"), json!(2))]);
}

#[test]
fn messages_move_to_the_dead_letter_queue_after_max_receives() {
    let responses = run(&[
        create("dlq"),
        create_with(
            "q",
            json!({"VisibilityTimeout": 0, "DeadLetterQueue": "dlq", "MaxReceiveCount": 2}),
        ),
        send("q", "poison"),
        send("q", "fine"),
        receive("q"),
        receive("q"),
        json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 2}),
        counts("q"),
        counts("dlq"),
        receive("dlq"),
    ]);
    assert_eq!(
        responses[6],
        json!({"ok": true, "Messages": [message("m-2", "r-3", "fine", 1, 0, 0)]})
    );
    assert_eq!(
        responses[7]["Counts"],
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(
        responses[8]["Counts"],
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(
        responses[9],
        json!({"ok": true, "Messages": [message("m-1", "r-4", "poison", 1, 0, 0)]})
    );
}

#[test]
fn dead_letter_move_ignores_the_message_limit_of_the_target_and_honors_its_retention() {
    let responses = run(&[
        create_with("dlq", json!({"RetentionSeconds": 100})),
        create_with(
            "q",
            json!({"DeadLetterQueue": "dlq", "MaxReceiveCount": 1, "VisibilityTimeout": 0}),
        ),
        send("q", "old"),
        receive("q"),
        tick(100),
        receive("q"),
        counts("dlq"),
    ]);
    assert_eq!(responses[5], json!({"ok": true, "Messages": []}));
    assert_eq!(
        responses[6]["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn deleting_a_dead_letter_queue_clears_the_configuration() {
    let responses = run(&[
        create("dlq"),
        create_with("q", json!({"DeadLetterQueue": "dlq", "MaxReceiveCount": 5})),
        json!({"Command": "DeleteQueue", "QueueName": "dlq"}),
        json!({"Command": "DeleteQueue", "QueueName": "dlq"}),
        counts("q"),
        create("dlq"),
        counts("dlq"),
    ]);
    assert_eq!(responses[3], failure("QueueDoesNotExist"));
    assert_eq!(
        responses[4]["Attributes"],
        json!({"VisibilityTimeout": 30, "DelaySeconds": 0, "RetentionSeconds": 345600,
               "Fifo": false, "ContentBasedDeduplication": false})
    );
    assert_eq!(
        responses[6]["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn handles_do_not_survive_deleting_and_recreating_the_queue() {
    let responses = run(&[
        create("q"),
        send("q", "x"),
        receive("q"),
        json!({"Command": "DeleteQueue", "QueueName": "q"}),
        create("q"),
        json!({"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": "r-1"}),
    ]);
    assert_eq!(responses[5], failure("ReceiptHandleExpired"));
}

#[test]
fn queue_is_limited_to_100000_messages() {
    let mut input = lines(&[create("q")]);
    let line = lines(&[send("q", "x")]);
    for _ in 0..100_001 {
        input.push_str(&line);
    }
    let responses = normalized(session(&input));
    assert_eq!(responses.len(), 100_002);
    assert_eq!(
        responses[100_000],
        json!({"ok": true, "MessageId": "m-100000"})
    );
    assert_eq!(responses[100_001], failure("LimitExceeded"));
}

#[test]
fn failed_and_duplicate_sends_do_not_use_up_message_ids() {
    let responses = run(&[
        create_with("q.fifo", json!({"Fifo": true})),
        json!({"Command": "SendMessage", "QueueName": "q.fifo", "Body": "x", "GroupId": "g"}),
        json!({"Command": "SendMessage", "QueueName": "q.fifo", "Body": "x", "GroupId": "g", "DeduplicationId": "d"}),
        json!({"Command": "SendMessage", "QueueName": "q.fifo", "Body": "x", "GroupId": "g", "DeduplicationId": "d"}),
        json!({"Command": "SendMessage", "QueueName": "q.fifo", "Body": "x", "GroupId": "g", "DeduplicationId": "e"}),
    ]);
    assert_eq!(responses[1], failure("MissingParameter"));
    assert_eq!(responses[2]["MessageId"], json!("m-1"));
    assert_eq!(responses[3]["MessageId"], json!("m-1"));
    assert_eq!(responses[4]["MessageId"], json!("m-2"));
}
