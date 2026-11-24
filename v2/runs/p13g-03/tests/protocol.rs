use std::io::{Cursor, Write};
use std::process::{Command, Stdio};

use serde_json::{Value, json};

/// Feeds `lines` to miniq and returns one parsed response per line.
fn run(lines: &[Value]) -> Vec<Value> {
    let input: String = lines.iter().map(|line| format!("{line}\n")).collect();
    run_raw(input.as_bytes())
}

fn run_raw(input: &[u8]) -> Vec<Value> {
    let mut output = Vec::new();
    miniq::serve(Cursor::new(input), &mut output).expect("in-memory I/O does not fail");
    String::from_utf8(output)
        .expect("responses are UTF-8")
        .lines()
        .map(|line| serde_json::from_str(line).expect("each response line is JSON"))
        .collect()
}

fn code(response: &Value) -> &str {
    assert_eq!(response["ok"], json!(false), "{response}");
    response["error"]["code"]
        .as_str()
        .expect("error has a code")
}

fn create(name: &str, attributes: Value) -> Value {
    json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attributes})
}

fn send(queue: &str, body: &str) -> Value {
    json!({"Command": "SendMessage", "QueueName": queue, "Body": body})
}

fn receive(queue: &str) -> Value {
    json!({"Command": "ReceiveMessage", "QueueName": queue})
}

fn tick(seconds: u64) -> Value {
    json!({"Command": "Tick", "Seconds": seconds})
}

fn counts(queue: &str) -> Value {
    json!({"Command": "GetQueueAttributes", "QueueName": queue})
}

fn ok() -> Value {
    json!({"ok": true})
}

#[test]
fn every_line_gets_a_response_and_bad_lines_do_not_stop_the_stream() {
    let responses = run_raw(b"\n[1]\n{\"Command\": 1}\n{\"Command\": \"Nope\"}\n\xff\xfe\n{\"Command\":\"Tick\",\"Seconds\":4}");
    let codes: Vec<&str> = responses[..5].iter().map(code).collect();
    assert_eq!(codes, ["InvalidRequest"; 5]);
    assert_eq!(responses[5], json!({"ok": true, "Now": 4}));
    assert_eq!(responses.len(), 6);
}

#[test]
fn binary_answers_over_stdin_and_exits_zero() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_miniq"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("miniq binary starts");
    child
        .stdin
        .take()
        .expect("stdin is piped")
        .write_all(b"{\"Command\":\"Tick\",\"Seconds\":1}\n{\"Command\":\"Tick\",\"Seconds\":2}")
        .expect("write to child");
    let output = child.wait_with_output().expect("child exits");
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).expect("UTF-8"),
        "{\"ok\":true,\"Now\":1}\n{\"ok\":true,\"Now\":3}\n"
    );
}

#[test]
fn create_queue_is_idempotent_only_for_equal_attributes() {
    let responses = run(&[
        create("jobs", json!({"DelaySeconds": 5})),
        create("jobs", json!({"DelaySeconds": 5, "VisibilityTimeout": 30})),
        create("jobs", json!({})),
        create("jobs.fifo", json!({})),
        create("plain", json!({"Fifo": true})),
        create("c", json!({"ContentBasedDeduplication": true})),
        create("d", json!({"DeadLetterQueue": "jobs"})),
        create(
            "e",
            json!({"DeadLetterQueue": "ghost", "MaxReceiveCount": 2}),
        ),
        create(
            "f.fifo",
            json!({"Fifo": true, "DeadLetterQueue": "jobs", "MaxReceiveCount": 2}),
        ),
    ]);
    assert_eq!(responses[0], ok());
    assert_eq!(responses[1], ok());
    let codes: Vec<&str> = responses[2..].iter().map(code).collect();
    assert_eq!(
        codes,
        [
            "QueueAlreadyExists",
            "QueueNameInvalid",
            "QueueNameInvalid",
            "InvalidParameterValue",
            "InvalidParameterValue",
            "QueueDoesNotExist",
            "InvalidParameterValue",
        ]
    );
}

#[test]
fn queue_attributes_and_counts_are_reported_whole() {
    let responses = run(&[
        create("jobs", json!({})),
        send("jobs", "a"),
        json!({"Command": "SendMessage", "QueueName": "jobs", "Body": "b", "DelaySeconds": 9}),
        receive("jobs"),
        counts("jobs"),
    ]);
    assert_eq!(
        responses[4],
        json!({
            "ok": true,
            "Attributes": {
                "VisibilityTimeout": 30,
                "DelaySeconds": 0,
                "RetentionSeconds": 345600,
                "Fifo": false,
                "ContentBasedDeduplication": false
            },
            "Counts": {"Visible": 0, "InFlight": 1, "Delayed": 1}
        })
    );
}

#[test]
fn receive_hides_message_until_visibility_timeout_and_keeps_its_place() {
    let responses = run(&[
        create("jobs", json!({})),
        send("jobs", "first"),
        send("jobs", "second"),
        receive("jobs"),
        receive("jobs"),
        tick(30),
        receive("jobs"),
    ]);
    assert_eq!(responses[1], json!({"ok": true, "MessageId": "m-1"}));
    assert_eq!(
        responses[3],
        json!({"ok": true, "Messages": [{
            "MessageId": "m-1", "ReceiptHandle": "r-1", "Body": "first",
            "ReceiveCount": 1, "SentAt": 0, "FirstReceivedAt": 0
        }]})
    );
    assert_eq!(responses[4]["Messages"][0]["MessageId"], json!("m-2"));
    assert_eq!(
        responses[6],
        json!({"ok": true, "Messages": [{
            "MessageId": "m-1", "ReceiptHandle": "r-3", "Body": "first",
            "ReceiveCount": 2, "SentAt": 0, "FirstReceivedAt": 0
        }]})
    );
}

#[test]
fn delay_applies_to_later_sends_only_and_ends_at_the_deadline() {
    let responses = run(&[
        create("jobs", json!({"DelaySeconds": 10})),
        send("jobs", "late"),
        json!({"Command": "SetQueueAttributes", "QueueName": "jobs", "Attributes": {"DelaySeconds": 0}}),
        send("jobs", "now"),
        receive("jobs"),
        tick(9),
        receive("jobs"),
        tick(1),
        receive("jobs"),
    ]);
    assert_eq!(responses[4]["Messages"][0]["MessageId"], json!("m-2"));
    assert_eq!(responses[6]["Messages"], json!([]));
    assert_eq!(responses[8]["Messages"][0]["MessageId"], json!("m-1"));
}

#[test]
fn receipt_handles_are_invalid_when_never_issued_and_expired_when_stale() {
    let responses = run(&[
        create("jobs", json!({})),
        create("other", json!({})),
        send("jobs", "a"),
        receive("jobs"),
        json!({"Command": "DeleteMessage", "QueueName": "jobs", "ReceiptHandle": "r-2"}),
        json!({"Command": "DeleteMessage", "QueueName": "jobs", "ReceiptHandle": "r-01"}),
        json!({"Command": "DeleteMessage", "QueueName": "other", "ReceiptHandle": "r-1"}),
        json!({"Command": "ChangeMessageVisibility", "QueueName": "jobs", "ReceiptHandle": "r-1", "VisibilityTimeout": 100}),
        tick(99),
        json!({"Command": "DeleteMessage", "QueueName": "jobs", "ReceiptHandle": "r-1"}),
        send("jobs", "b"),
        json!({"Command": "DeleteMessage", "QueueName": "jobs", "ReceiptHandle": "r-1"}),
    ]);
    assert_eq!(code(&responses[4]), "ReceiptHandleInvalid");
    assert_eq!(code(&responses[5]), "ReceiptHandleInvalid");
    assert_eq!(code(&responses[6]), "ReceiptHandleExpired");
    assert_eq!(responses[7], ok());
    assert_eq!(responses[9], ok());
    assert_eq!(code(&responses[11]), "ReceiptHandleExpired");
}

#[test]
fn zero_visibility_timeout_makes_the_handle_unusable() {
    let responses = run(&[
        create("jobs", json!({})),
        send("jobs", "a"),
        json!({"Command": "ReceiveMessage", "QueueName": "jobs", "VisibilityTimeout": 0}),
        json!({"Command": "DeleteMessage", "QueueName": "jobs", "ReceiptHandle": "r-1"}),
        counts("jobs"),
    ]);
    assert_eq!(code(&responses[3]), "ReceiptHandleExpired");
    assert_eq!(
        responses[4]["Counts"],
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn fifo_delivers_one_message_per_group_in_order() {
    let fifo = |body: &str, group: &str, dedup: &str| {
        json!({"Command": "SendMessage", "QueueName": "q.fifo", "Body": body,
               "GroupId": group, "DeduplicationId": dedup})
    };
    let responses = run(&[
        create("q.fifo", json!({"Fifo": true})),
        fifo("a1", "A", "d1"),
        fifo("a2", "A", "d2"),
        fifo("b1", "B", "d3"),
        json!({"Command": "ReceiveMessage", "QueueName": "q.fifo", "MaxMessages": 10}),
        json!({"Command": "ReceiveMessage", "QueueName": "q.fifo", "MaxMessages": 10}),
    ]);
    assert_eq!(
        responses[3],
        json!({"ok": true, "MessageId": "m-3", "SequenceNumber": 3})
    );
    let bodies: Vec<&Value> = responses[4]["Messages"]
        .as_array()
        .expect("messages array")
        .iter()
        .map(|message| &message["Body"])
        .collect();
    assert_eq!(bodies, [&json!("a1"), &json!("b1")]);
    assert_eq!(responses[5]["Messages"], json!([]));
}

#[test]
fn fifo_duplicate_window_is_not_extended_by_duplicates() {
    let dup = |body: &str| {
        json!({"Command": "SendMessage", "QueueName": "q.fifo", "Body": body,
               "GroupId": "g", "DeduplicationId": "same"})
    };
    let responses = run(&[
        create("q.fifo", json!({"Fifo": true})),
        dup("one"),
        tick(299),
        dup("two"),
        tick(1),
        dup("three"),
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
}

#[test]
fn content_based_deduplication_uses_the_body() {
    let message = |body: &str| json!({"Command": "SendMessage", "QueueName": "q.fifo", "Body": body, "GroupId": "g"});
    let responses = run(&[
        create(
            "q.fifo",
            json!({"Fifo": true, "ContentBasedDeduplication": true}),
        ),
        message("x"),
        message("x"),
        message("y"),
    ]);
    assert_eq!(responses[2], responses[1]);
    assert_eq!(responses[3]["SequenceNumber"], json!(2));
}

#[test]
fn send_rules_for_each_queue_type() {
    let responses = run(&[
        create("std", json!({})),
        create("q.fifo", json!({"Fifo": true})),
        json!({"Command": "SendMessage", "QueueName": "std", "Body": "x", "GroupId": "g"}),
        json!({"Command": "SendMessage", "QueueName": "q.fifo", "Body": "x", "DelaySeconds": 1}),
        json!({"Command": "SendMessage", "QueueName": "q.fifo", "Body": "x"}),
        json!({"Command": "SendMessage", "QueueName": "q.fifo", "Body": "x", "GroupId": "g"}),
        json!({"Command": "SendMessage", "QueueName": "ghost", "Body": ""}),
        json!({"Command": "SendMessage", "QueueName": "ghost", "Body": "x"}),
    ]);
    let codes: Vec<&str> = responses[2..].iter().map(code).collect();
    assert_eq!(
        codes,
        [
            "UnsupportedOperation",
            "UnsupportedOperation",
            "MissingParameter",
            "MissingParameter",
            "InvalidParameterValue",
            "QueueDoesNotExist",
        ]
    );
}

#[test]
fn dead_letter_move_resets_receive_count_and_first_receipt() {
    let responses = run(&[
        create("dead", json!({})),
        create(
            "main",
            json!({"DeadLetterQueue": "dead", "MaxReceiveCount": 1}),
        ),
        send("main", "poison"),
        json!({"Command": "ReceiveMessage", "QueueName": "main", "VisibilityTimeout": 0}),
        receive("main"),
        counts("main"),
        json!({"Command": "ReceiveMessage", "QueueName": "dead"}),
    ]);
    assert_eq!(responses[4]["Messages"], json!([]));
    assert_eq!(
        responses[5]["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(
        responses[6]["Messages"],
        json!([{
            "MessageId": "m-1", "ReceiptHandle": "r-2", "Body": "poison",
            "ReceiveCount": 1, "SentAt": 0, "FirstReceivedAt": 0
        }])
    );
}

#[test]
fn deleting_a_dead_letter_queue_unsets_the_configuration() {
    let responses = run(&[
        create("dead", json!({})),
        create(
            "main",
            json!({"DeadLetterQueue": "dead", "MaxReceiveCount": 3}),
        ),
        counts("main"),
        json!({"Command": "DeleteQueue", "QueueName": "dead"}),
        counts("main"),
    ]);
    assert_eq!(responses[2]["Attributes"]["DeadLetterQueue"], json!("dead"));
    assert_eq!(responses[2]["Attributes"]["MaxReceiveCount"], json!(3));
    assert_eq!(responses[4]["Attributes"].get("DeadLetterQueue"), None);
    assert_eq!(responses[4]["Attributes"].get("MaxReceiveCount"), None);
}

#[test]
fn retention_removes_messages_at_once_when_lowered() {
    let responses = run(&[
        create("jobs", json!({})),
        send("jobs", "a"),
        tick(60),
        counts("jobs"),
        json!({"Command": "SetQueueAttributes", "QueueName": "jobs", "Attributes": {"RetentionSeconds": 60}}),
        counts("jobs"),
    ]);
    assert_eq!(responses[3]["Counts"]["Visible"], json!(1));
    assert_eq!(responses[5]["Counts"]["Visible"], json!(0));
}

#[test]
fn list_queues_sorts_by_byte_value_and_filters_by_prefix() {
    let responses = run(&[
        create("b", json!({})),
        create("a.fifo", json!({"Fifo": true})),
        create("a-1", json!({})),
        create("A", json!({})),
        create("_x", json!({})),
        json!({"Command": "ListQueues"}),
        json!({"Command": "ListQueues", "Prefix": "a"}),
    ]);
    assert_eq!(
        responses[5],
        json!({"ok": true, "QueueNames": ["A", "_x", "a-1", "a.fifo", "b"]})
    );
    assert_eq!(responses[6]["QueueNames"], json!(["a-1", "a.fifo"]));
}

#[test]
fn purge_keeps_sequence_numbers_and_deduplication() {
    let message = json!({"Command": "SendMessage", "QueueName": "q.fifo", "Body": "x",
                         "GroupId": "g", "DeduplicationId": "d"});
    let other = json!({"Command": "SendMessage", "QueueName": "q.fifo", "Body": "y",
                       "GroupId": "g", "DeduplicationId": "e"});
    let responses = run(&[
        create("q.fifo", json!({"Fifo": true})),
        message.clone(),
        json!({"Command": "PurgeQueue", "QueueName": "q.fifo"}),
        message,
        other,
    ]);
    assert_eq!(responses[3], responses[1]);
    assert_eq!(responses[4]["SequenceNumber"], json!(2));
}

#[test]
fn a_queue_holds_at_most_100000_messages() {
    let mut lines = vec![create("jobs", json!({}))];
    lines.extend((0..100_001).map(|_| send("jobs", "x")));
    let responses = run(&lines);
    assert_eq!(
        responses[100_000],
        json!({"ok": true, "MessageId": "m-100000"})
    );
    assert_eq!(code(&responses[100_001]), "LimitExceeded");
}
