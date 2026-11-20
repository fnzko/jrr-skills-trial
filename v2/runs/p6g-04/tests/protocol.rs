//! End-to-end tests that drive the service through its line protocol.

use std::io::{Cursor, Write};
use std::process::{Command, Stdio};

use serde_json::{Value, json};

/// Runs raw input through the service and returns the response lines.
fn run_raw(input: &[u8]) -> Vec<Value> {
    let mut output = Vec::new();
    miniq::run(Cursor::new(input), &mut output).expect("in-memory io does not fail");
    String::from_utf8(output)
        .expect("responses are UTF-8")
        .lines()
        .map(|line| serde_json::from_str(line).expect("each response is one JSON value"))
        .collect()
}

fn run(commands: &[Value]) -> Vec<Value> {
    let input: String = commands.iter().map(|c| format!("{c}\n")).collect();
    run_raw(input.as_bytes())
}

fn error_code(response: &Value) -> &str {
    assert_eq!(response["ok"], json!(false), "{response}");
    response["error"]["code"]
        .as_str()
        .expect("error has a code")
}

/// The error code of a failed response, or "ok" for a successful one.
fn outcome(response: &Value) -> &str {
    if response["ok"] == json!(true) {
        "ok"
    } else {
        error_code(response)
    }
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

fn attributes(queue: &str) -> Value {
    json!({"Command": "GetQueueAttributes", "QueueName": queue})
}

fn bodies(response: &Value) -> Vec<&str> {
    response["Messages"]
        .as_array()
        .expect("a receive result has Messages")
        .iter()
        .map(|m| m["Body"].as_str().expect("messages have a body"))
        .collect()
}

#[test]
fn every_line_gets_one_response_including_blank_and_unterminated_ones() {
    let responses =
        run_raw(b"\n{\"Command\":\"Tick\",\"Seconds\":2}\n{\"Command\":\"Tick\",\"Seconds\":3}");
    let codes: Vec<_> = responses.iter().map(|r| r["ok"].clone()).collect();
    assert_eq!(codes, [json!(false), json!(true), json!(true)]);
    assert_eq!(error_code(&responses[0]), "InvalidRequest");
    assert_eq!(responses[2], json!({"ok": true, "Now": 5}));
}

#[test]
fn an_empty_input_produces_no_output() {
    assert_eq!(run_raw(b""), Vec::<Value>::new());
}

#[test]
fn request_shape_errors_are_invalid_request() {
    let lines: [&[u8]; 8] = [
        b"not json",
        b"[1]",
        b"{}",
        br#"{"Command": 5}"#,
        br#"{"Command": null}"#,
        br#"{"Command": "Nope"}"#,
        b"{\"Command\": \"Tick\", \"Seconds\": 1} x",
        b"{\"Command\": \"Tick\", \"Seconds\": 1, \"x\": \"\xff\"}",
    ];
    for line in lines {
        let mut input = line.to_vec();
        input.push(b'\n');
        let responses = run_raw(&input);
        assert_eq!(
            error_code(&responses[0]),
            "InvalidRequest",
            "{:?}",
            String::from_utf8_lossy(line)
        );
    }
}

#[test]
fn a_bad_line_does_not_stop_later_lines() {
    let responses = run_raw(b"garbage\n{\"Command\":\"Tick\",\"Seconds\":1}\n");
    assert_eq!(responses.len(), 2);
    assert_eq!(responses[1], json!({"ok": true, "Now": 1}));
}

#[test]
fn error_precedence_follows_the_spec_order() {
    let cases = [
        // missing parameters beat invalid values
        (
            json!({"Command": "SendMessage", "Body": 5}),
            "MissingParameter",
        ),
        (
            json!({"Command": "SendMessage", "QueueName": "q"}),
            "MissingParameter",
        ),
        (
            json!({"Command": "SetQueueAttributes", "QueueName": "q"}),
            "MissingParameter",
        ),
        (
            json!({"Command": "Tick", "Seconds": null}),
            "MissingParameter",
        ),
        // invalid values beat the queue lookup
        (
            json!({"Command": "SendMessage", "QueueName": "nope", "Body": ""}),
            "InvalidParameterValue",
        ),
        (
            json!({"Command": "SendMessage", "QueueName": 7, "Body": "x"}),
            "InvalidParameterValue",
        ),
        // lookup beats command rules, whatever the name looks like
        (
            json!({"Command": "SendMessage", "QueueName": "bad name!", "Body": "x"}),
            "QueueDoesNotExist",
        ),
        (
            json!({"Command": "DeleteMessage", "QueueName": "nope", "ReceiptHandle": "junk"}),
            "QueueDoesNotExist",
        ),
        // CreateQueue checks the name before the other parameters
        (create("bad name", json!(5)), "QueueNameInvalid"),
        (
            json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": 5}),
            "InvalidParameterValue",
        ),
        (create("q", json!({"Unknown": 1})), "InvalidParameterValue"),
        (
            create("q", json!({"VisibilityTimeout": 43201})),
            "InvalidParameterValue",
        ),
        (
            create("q", json!({"VisibilityTimeout": 30.0})),
            "InvalidParameterValue",
        ),
        (
            create("q", json!({"DelaySeconds": "3"})),
            "InvalidParameterValue",
        ),
        (
            create("q", json!({"RetentionSeconds": 59})),
            "InvalidParameterValue",
        ),
        (
            create("q", json!({"MaxReceiveCount": 0})),
            "InvalidParameterValue",
        ),
        (create("q", json!({"Fifo": 1})), "InvalidParameterValue"),
        // value checks beat the numbered rules of the command
        (
            create("q.fifo", json!({"DelaySeconds": 901})),
            "InvalidParameterValue",
        ),
        (tick(1_209_601), "InvalidParameterValue"),
    ];
    for (command, code) in cases {
        let responses = run(std::slice::from_ref(&command));
        assert_eq!(error_code(&responses[0]), code, "{command}");
    }
}

#[test]
fn null_parameters_and_attributes_are_absent() {
    let responses = run(&[
        create("q", json!({"VisibilityTimeout": null, "Bogus": null})),
        json!({"Command": "ListQueues", "Prefix": null}),
        json!({"Command": "Tick", "Seconds": 0, "Extra": [1]}),
    ]);
    assert_eq!(responses[0], json!({"ok": true}));
    assert_eq!(responses[1], json!({"ok": true, "QueueNames": ["q"]}));
    assert_eq!(responses[2], json!({"ok": true, "Now": 0}));
}

#[test]
fn create_queue_rules_apply_in_order() {
    let responses = run(&[
        create("a.fifo", json!({})),
        create("a", json!({"Fifo": true})),
        create("a", json!({"ContentBasedDeduplication": true})),
        create("a", json!({"MaxReceiveCount": 3})),
        create(
            "a",
            json!({"DeadLetterQueue": "missing", "MaxReceiveCount": 3}),
        ),
        create("dlq", json!({})),
        create(
            "a.fifo",
            json!({"Fifo": true, "DeadLetterQueue": "dlq", "MaxReceiveCount": 3}),
        ),
        create("a", json!({"DeadLetterQueue": "dlq", "MaxReceiveCount": 3})),
        attributes("a"),
    ]);
    let outcomes: Vec<_> = responses.iter().map(outcome).collect();
    assert_eq!(
        outcomes,
        [
            "QueueNameInvalid",
            "QueueNameInvalid",
            "InvalidParameterValue",
            "InvalidParameterValue",
            "QueueDoesNotExist",
            "ok",
            "InvalidParameterValue",
            "ok",
            "ok",
        ]
    );
    assert_eq!(
        responses[8],
        json!({
            "ok": true,
            "Attributes": {
                "VisibilityTimeout": 30, "DelaySeconds": 0, "RetentionSeconds": 345600,
                "Fifo": false, "ContentBasedDeduplication": false,
                "DeadLetterQueue": "dlq", "MaxReceiveCount": 3
            },
            "Counts": {"Visible": 0, "InFlight": 0, "Delayed": 0}
        })
    );
}

#[test]
fn creating_an_existing_queue_succeeds_only_with_equal_attributes() {
    let responses = run(&[
        create("a", json!({"DelaySeconds": 5})),
        create("a", json!({"DelaySeconds": 5, "VisibilityTimeout": 30})),
        create("a", json!({})),
        create(
            "a",
            json!({"DelaySeconds": 5, "DeadLetterQueue": "gone", "MaxReceiveCount": 2}),
        ),
    ]);
    let outcomes: Vec<_> = responses.iter().map(outcome).collect();
    assert_eq!(
        outcomes,
        ["ok", "ok", "QueueAlreadyExists", "QueueAlreadyExists"]
    );
}

#[test]
fn set_queue_attributes_validates_then_merges() {
    let set = |attributes: Value| json!({"Command": "SetQueueAttributes", "QueueName": "a", "Attributes": attributes});
    let responses = run(&[
        create("a", json!({})),
        create("b.fifo", json!({"Fifo": true})),
        create("c", json!({})),
        set(json!({"Fifo": false})),
        set(json!({"ContentBasedDeduplication": true})),
        set(json!({"DeadLetterQueue": "c"})),
        set(json!({"DeadLetterQueue": "nope", "MaxReceiveCount": 2})),
        set(json!({"DeadLetterQueue": "a", "MaxReceiveCount": 2})),
        set(json!({"DeadLetterQueue": "b.fifo", "MaxReceiveCount": 2})),
        set(json!({"DeadLetterQueue": "c", "MaxReceiveCount": 2, "DelaySeconds": 7})),
        set(json!({"VisibilityTimeout": 9})),
        json!({"Command": "SetQueueAttributes", "QueueName": "nope", "Attributes": {}}),
        attributes("a"),
    ]);
    let outcomes: Vec<_> = responses.iter().map(outcome).collect();
    assert_eq!(
        outcomes,
        [
            "ok",
            "ok",
            "ok",
            "UnsupportedOperation",
            "InvalidParameterValue",
            "InvalidParameterValue",
            "QueueDoesNotExist",
            "InvalidParameterValue",
            "InvalidParameterValue",
            "ok",
            "ok",
            "QueueDoesNotExist",
            "ok",
        ]
    );
    assert_eq!(
        responses[12]["Attributes"],
        json!({
            "VisibilityTimeout": 9, "DelaySeconds": 7, "RetentionSeconds": 345600,
            "Fifo": false, "ContentBasedDeduplication": false,
            "DeadLetterQueue": "c", "MaxReceiveCount": 2
        })
    );
}

#[test]
fn list_queues_sorts_by_bytes_and_filters_by_prefix() {
    let names = ["b", "a_", "a-", "a.fifo", "a0", "aB", "ab"];
    let mut commands: Vec<Value> = names
        .iter()
        .map(|name| {
            let fifo = name.ends_with(".fifo");
            create(name, json!({"Fifo": fifo}))
        })
        .collect();
    commands.push(json!({"Command": "ListQueues"}));
    commands.push(json!({"Command": "ListQueues", "Prefix": "a"}));
    commands.push(json!({"Command": "ListQueues", "Prefix": "a."}));
    commands.push(json!({"Command": "ListQueues", "Prefix": "zz"}));
    let responses = run(&commands);
    let listed = &responses[names.len()..];
    assert_eq!(
        listed[0]["QueueNames"],
        json!(["a-", "a.fifo", "a0", "aB", "a_", "ab", "b"])
    );
    assert_eq!(
        listed[1]["QueueNames"],
        json!(["a-", "a.fifo", "a0", "aB", "a_", "ab"])
    );
    assert_eq!(listed[2]["QueueNames"], json!(["a.fifo"]));
    assert_eq!(listed[3]["QueueNames"], json!([]));
}

#[test]
fn a_message_is_delivered_hidden_and_visible_again_after_its_timeout() {
    let responses = run(&[
        create("q", json!({"VisibilityTimeout": 10})),
        send("q", "one"),
        receive("q"),
        receive("q"),
        tick(9),
        receive("q"),
        tick(1),
        receive("q"),
    ]);
    let message = |handle: &str, count: u64| {
        json!({"ok": true, "Messages": [{
            "MessageId": "m-1", "ReceiptHandle": handle, "Body": "one",
            "ReceiveCount": count, "SentAt": 0, "FirstReceivedAt": 0
        }]})
    };
    assert_eq!(responses[1], json!({"ok": true, "MessageId": "m-1"}));
    assert_eq!(responses[2], message("r-1", 1));
    assert_eq!(responses[3], json!({"ok": true, "Messages": []}));
    assert_eq!(responses[5], json!({"ok": true, "Messages": []}));
    assert_eq!(responses[7], {
        let mut again = message("r-2", 2);
        again["Messages"][0]["FirstReceivedAt"] = json!(0);
        again
    });
}

#[test]
fn receive_honours_max_messages_and_a_per_call_visibility_timeout() {
    let responses = run(&[
        create("q", json!({})),
        send("q", "a"),
        send("q", "b"),
        send("q", "c"),
        json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 2, "VisibilityTimeout": 0}),
        json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 10}),
    ]);
    assert_eq!(bodies(&responses[4]), ["a", "b"]);
    assert_eq!(bodies(&responses[5]), ["a", "b", "c"]);
    let handles: Vec<_> = responses[4..]
        .iter()
        .flat_map(|r| r["Messages"].as_array().unwrap())
        .map(|m| m["ReceiptHandle"].as_str().unwrap())
        .collect();
    assert_eq!(handles, ["r-1", "r-2", "r-3", "r-4", "r-5"]);
}

#[test]
fn receipt_handles_are_invalid_or_expired_by_the_spec_rules() {
    let delete = |queue: &str, handle: &str| json!({"Command": "DeleteMessage", "QueueName": queue, "ReceiptHandle": handle});
    let change = |handle: &str, timeout: u64| json!({"Command": "ChangeMessageVisibility", "QueueName": "q", "ReceiptHandle": handle, "VisibilityTimeout": timeout});
    let responses = run(&[
        create("q", json!({"VisibilityTimeout": 5})),
        create("other", json!({})),
        send("q", "a"),
        send("q", "b"),
        receive("q"),        // r-1 for a
        delete("q", "r-01"), // never issued
        delete("q", "R-1"),
        delete("q", "r-1 "),
        delete("q", "r-9"),
        delete("other", "r-1"), // issued, but for another queue
        tick(5),
        delete("q", "r-1"), // timeout ended
        receive("q"),       // r-2 for a again
        delete("q", "r-1"), // delivered again since
        change("r-2", 0),   // visible at once
        delete("q", "r-2"), // so the handle is dead
        receive("q"),       // r-3 for a
        change("r-3", 100),
        tick(99),
        change("r-3", 1), // still in flight, now ends at 105
        delete("q", "r-3"),
        delete("q", "r-3"), // gone
        receive("q"),       // r-4 for b
    ]);
    let outcomes: Vec<_> = responses[5..].iter().map(outcome).collect();
    assert_eq!(
        outcomes,
        [
            "ReceiptHandleInvalid",
            "ReceiptHandleInvalid",
            "ReceiptHandleInvalid",
            "ReceiptHandleInvalid",
            "ReceiptHandleExpired",
            "ok",
            "ReceiptHandleExpired",
            "ok",
            "ReceiptHandleExpired",
            "ok",
            "ReceiptHandleExpired",
            "ok",
            "ok",
            "ok",
            "ok",
            "ok",
            "ReceiptHandleExpired",
            "ok",
        ]
    );
    assert_eq!(bodies(&responses[22]), ["b"]);
    assert_eq!(responses[22]["Messages"][0]["ReceiptHandle"], json!("r-4"));
}

#[test]
fn delay_hides_messages_until_it_has_passed_and_the_send_can_override_the_queue_default() {
    let responses = run(&[
        create("q", json!({"DelaySeconds": 10})),
        send("q", "queue delay"),
        json!({"Command": "SendMessage", "QueueName": "q", "Body": "no delay", "DelaySeconds": 0}),
        json!({"Command": "SendMessage", "QueueName": "q", "Body": "long", "DelaySeconds": 20}),
        attributes("q"),
        receive("q"),
        tick(10),
        attributes("q"),
    ]);
    assert_eq!(
        responses[4]["Counts"],
        json!({"Visible": 1, "InFlight": 0, "Delayed": 2})
    );
    assert_eq!(bodies(&responses[5]), ["no delay"]);
    assert_eq!(
        responses[7]["Counts"],
        json!({"Visible": 1, "InFlight": 1, "Delayed": 1})
    );
}

#[test]
fn retention_removes_messages_at_once_and_is_not_undone_by_raising_it() {
    let set = |seconds: u64| json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"RetentionSeconds": seconds}});
    let responses = run(&[
        create(
            "q",
            json!({"RetentionSeconds": 100, "VisibilityTimeout": 1000}),
        ),
        send("q", "old"),
        receive("q"),
        tick(60),
        send("q", "young"),
        tick(40), // now 100: "old" is gone, even though it is in flight
        attributes("q"),
        tick(20),
        set(1000), // raising does not bring anything back
        attributes("q"),
        set(60), // "young" was sent at 60, now is 120: removed at once
        attributes("q"),
    ]);
    assert_eq!(
        responses[6]["Counts"],
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(
        responses[9]["Counts"],
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(
        responses[11]["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn lowering_retention_then_raising_it_in_the_next_command_still_removes_messages() {
    let set = |seconds: u64| json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"RetentionSeconds": seconds}});
    let responses = run(&[
        create("q", json!({})),
        send("q", "a"),
        tick(100),
        set(60),
        set(1000),
        attributes("q"),
    ]);
    assert_eq!(
        responses[5]["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn standard_queues_reject_fifo_parameters() {
    let responses = run(&[
        create("q", json!({})),
        json!({"Command": "SendMessage", "QueueName": "q", "Body": "x", "GroupId": "g"}),
        json!({"Command": "SendMessage", "QueueName": "q", "Body": "x", "DeduplicationId": "d"}),
        send("q", &"x".repeat(1025)),
        send("q", ""),
        send("q", &"\u{e9}".repeat(512)),
        send("q", &"\u{e9}".repeat(513)),
    ]);
    let outcomes: Vec<_> = responses.iter().map(outcome).collect();
    assert_eq!(
        outcomes,
        [
            "ok",
            "UnsupportedOperation",
            "UnsupportedOperation",
            "InvalidParameterValue",
            "InvalidParameterValue",
            "ok",
            "InvalidParameterValue"
        ]
    );
}

fn fifo_send(body: &str, group: &str, dedup: &str) -> Value {
    json!({"Command": "SendMessage", "QueueName": "q.fifo", "Body": body, "GroupId": group, "DeduplicationId": dedup})
}

#[test]
fn fifo_send_rules_apply_in_order() {
    let responses = run(&[
        create("q.fifo", json!({"Fifo": true})),
        json!({"Command": "SendMessage", "QueueName": "q.fifo", "Body": "x", "DelaySeconds": 1}),
        json!({"Command": "SendMessage", "QueueName": "q.fifo", "Body": "x", "DeduplicationId": "d"}),
        json!({"Command": "SendMessage", "QueueName": "q.fifo", "Body": "x", "GroupId": "g"}),
        json!({"Command": "SendMessage", "QueueName": "q.fifo", "Body": "x", "GroupId": "bad group", "DeduplicationId": "d"}),
        fifo_send("x", "g", "d"),
    ]);
    let outcomes: Vec<_> = responses.iter().map(outcome).collect();
    assert_eq!(
        outcomes,
        [
            "ok",
            "UnsupportedOperation",
            "MissingParameter",
            "MissingParameter",
            "InvalidParameterValue",
            "ok"
        ]
    );
    assert_eq!(
        responses[5],
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
}

#[test]
fn fifo_deduplication_window_is_300_seconds_and_is_not_extended_by_duplicates() {
    let responses = run(&[
        create("q.fifo", json!({"Fifo": true})),
        fifo_send("a", "g1", "d"),
        tick(299),
        fifo_send("b", "g2", "d"), // duplicate, original ids come back
        tick(1),                   // 300 after the first send
        fifo_send("c", "g1", "d"), // accepted, opens a new window
        fifo_send("e", "g1", "other"),
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
    assert_eq!(
        responses[6],
        json!({"ok": true, "MessageId": "m-3", "SequenceNumber": 3})
    );
}

#[test]
fn content_based_deduplication_uses_the_body_and_explicit_ids_win() {
    let content = |body: &str| json!({"Command": "SendMessage", "QueueName": "q.fifo", "Body": body, "GroupId": "g"});
    let responses = run(&[
        create(
            "q.fifo",
            json!({"Fifo": true, "ContentBasedDeduplication": true}),
        ),
        content("same"),
        content("same"),
        json!({"Command": "SendMessage", "QueueName": "q.fifo", "Body": "same", "GroupId": "g", "DeduplicationId": "explicit"}),
        content("different"),
    ]);
    assert_eq!(
        responses[1],
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
    assert_eq!(responses[2], responses[1]);
    assert_eq!(
        responses[3],
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2})
    );
    assert_eq!(
        responses[4],
        json!({"ok": true, "MessageId": "m-3", "SequenceNumber": 3})
    );
}

#[test]
fn deduplication_ids_outlive_deleted_and_purged_messages() {
    let responses = run(&[
        create("q.fifo", json!({"Fifo": true})),
        fifo_send("a", "g", "d"),
        receive("q.fifo"),
        json!({"Command": "DeleteMessage", "QueueName": "q.fifo", "ReceiptHandle": "r-1"}),
        fifo_send("a", "g", "d"),
        json!({"Command": "PurgeQueue", "QueueName": "q.fifo"}),
        fifo_send("a", "g", "d"),
        fifo_send("b", "g", "e"),
    ]);
    assert_eq!(responses[4], responses[1]);
    assert_eq!(responses[6], responses[1]);
    // purging keeps counting sequence numbers
    assert_eq!(
        responses[7],
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2})
    );
}

#[test]
fn fifo_delivers_one_message_per_group_in_order_and_blocks_groups_with_earlier_messages() {
    let responses = run(&[
        create("q.fifo", json!({"Fifo": true, "VisibilityTimeout": 10})),
        fifo_send("a1", "A", "1"),
        fifo_send("b1", "B", "2"),
        fifo_send("a2", "A", "3"),
        fifo_send("c1", "C", "4"),
        json!({"Command": "ReceiveMessage", "QueueName": "q.fifo", "MaxMessages": 10}),
        json!({"Command": "ReceiveMessage", "QueueName": "q.fifo", "MaxMessages": 10}),
        tick(10),
        json!({"Command": "DeleteMessage", "QueueName": "q.fifo", "ReceiptHandle": "r-1"}),
        json!({"Command": "ReceiveMessage", "QueueName": "q.fifo", "MaxMessages": 10}),
    ]);
    assert_eq!(bodies(&responses[5]), ["a1", "b1", "c1"]);
    assert_eq!(
        responses[5]["Messages"][1],
        json!({
            "MessageId": "m-2", "ReceiptHandle": "r-2", "Body": "b1", "ReceiveCount": 1,
            "SentAt": 0, "FirstReceivedAt": 0, "GroupId": "B", "SequenceNumber": 2
        })
    );
    assert_eq!(bodies(&responses[6]), Vec::<&str>::new());
    // a1 timed out and was not deleted in time, so it blocks a2 again
    assert_eq!(responses[8]["ok"], json!(false));
    assert_eq!(bodies(&responses[9]), ["a1", "b1", "c1"]);
}

#[test]
fn messages_over_the_receive_count_move_to_the_dead_letter_queue_with_a_fresh_history() {
    let responses = run(&[
        create("dlq", json!({"RetentionSeconds": 100})),
        create(
            "q",
            json!({"VisibilityTimeout": 0, "DeadLetterQueue": "dlq", "MaxReceiveCount": 2}),
        ),
        send("q", "poison"),
        send("q", "fine"),
        receive("q"),
        receive("q"),
        json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 5}),
        attributes("dlq"),
        json!({"Command": "ReceiveMessage", "QueueName": "dlq"}),
    ]);
    assert_eq!(bodies(&responses[4]), ["poison"]);
    assert_eq!(bodies(&responses[5]), ["poison"]);
    // poison has been received twice: it moves, and "fine" is delivered in its place
    assert_eq!(bodies(&responses[6]), ["fine"]);
    assert_eq!(
        responses[7]["Counts"],
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(
        responses[8]["Messages"][0],
        json!({
            "MessageId": "m-1", "ReceiptHandle": "r-4", "Body": "poison", "ReceiveCount": 1,
            "SentAt": 0, "FirstReceivedAt": 0
        })
    );
}

#[test]
fn a_dead_lettered_message_keeps_its_age_under_the_dead_letter_retention() {
    let responses = run(&[
        create("dlq", json!({"RetentionSeconds": 60})),
        create(
            "q",
            json!({"VisibilityTimeout": 0, "DeadLetterQueue": "dlq", "MaxReceiveCount": 1}),
        ),
        send("q", "old"),
        receive("q"),
        tick(70),
        send("q", "new"),
        receive("q"),
        attributes("dlq"),
    ]);
    // "old" moves at 70 seconds of age, which is past the 60 seconds the
    // dead-letter queue retains, so it is gone on arrival.
    assert_eq!(bodies(&responses[6]), ["new"]);
    assert_eq!(
        responses[7]["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn fifo_dead_letter_moves_take_new_sequence_numbers() {
    let responses = run(&[
        json!({"Command": "CreateQueue", "QueueName": "d.fifo", "Attributes": {"Fifo": true}}),
        json!({"Command": "CreateQueue", "QueueName": "q.fifo", "Attributes": {"Fifo": true, "VisibilityTimeout": 0, "DeadLetterQueue": "d.fifo", "MaxReceiveCount": 1}}),
        fifo_send("a", "g", "1"),
        receive("q.fifo"),
        receive("q.fifo"),
        json!({"Command": "ReceiveMessage", "QueueName": "d.fifo"}),
    ]);
    // the first receive on q.fifo is the single delivery, the second moves the message
    assert_eq!(responses[4], json!({"ok": true, "Messages": []}));
    assert_eq!(responses[5]["Messages"][0]["GroupId"], json!("g"));
    assert_eq!(responses[5]["Messages"][0]["SequenceNumber"], json!(1));
}

#[test]
fn deleting_a_queue_unconfigures_its_dependents_and_a_new_queue_starts_empty() {
    let responses = run(&[
        create("dlq", json!({})),
        create("q", json!({"DeadLetterQueue": "dlq", "MaxReceiveCount": 5})),
        send("dlq", "x"),
        json!({"Command": "DeleteQueue", "QueueName": "dlq"}),
        attributes("q"),
        json!({"Command": "DeleteQueue", "QueueName": "dlq"}),
        create("dlq", json!({})),
        attributes("dlq"),
    ]);
    assert_eq!(responses[3], json!({"ok": true}));
    assert_eq!(
        responses[4]["Attributes"],
        json!({"VisibilityTimeout": 30, "DelaySeconds": 0, "RetentionSeconds": 345600, "Fifo": false, "ContentBasedDeduplication": false})
    );
    assert_eq!(error_code(&responses[5]), "QueueDoesNotExist");
    assert_eq!(
        responses[7]["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn purge_removes_messages_in_every_state() {
    let responses = run(&[
        create("q", json!({})),
        send("q", "inflight"),
        receive("q"),
        json!({"Command": "SendMessage", "QueueName": "q", "Body": "delayed", "DelaySeconds": 5}),
        send("q", "visible"),
        json!({"Command": "PurgeQueue", "QueueName": "q"}),
        attributes("q"),
        json!({"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": "r-1"}),
    ]);
    assert_eq!(
        responses[6]["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(error_code(&responses[7]), "ReceiptHandleExpired");
}

#[test]
fn a_thousand_queues_is_the_limit_and_failures_change_nothing() {
    let mut commands: Vec<Value> = (0..1000)
        .map(|i| create(&format!("q{i}"), json!({})))
        .collect();
    commands.push(create("one-too-many", json!({})));
    commands.push(create("q0", json!({})));
    commands.push(json!({"Command": "ListQueues", "Prefix": "one"}));
    let responses = run(&commands);
    assert!(responses[..1000].iter().all(|r| r["ok"] == json!(true)));
    assert_eq!(error_code(&responses[1000]), "LimitExceeded");
    assert_eq!(responses[1001], json!({"ok": true}));
    assert_eq!(responses[1002], json!({"ok": true, "QueueNames": []}));
}

#[test]
fn a_queue_holds_100000_messages_and_filling_it_is_fast() {
    let mut commands = vec![create("q", json!({}))];
    commands.extend((0..100_000).map(|i| send("q", &i.to_string())));
    commands.push(send("q", "overflow"));
    commands.push(json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 1}));
    commands.push(attributes("q"));
    let started = std::time::Instant::now();
    let responses = run(&commands);
    assert!(
        started.elapsed().as_secs() < 20,
        "took {:?}",
        started.elapsed()
    );
    let last = responses.len();
    assert_eq!(error_code(&responses[last - 3]), "LimitExceeded");
    assert_eq!(bodies(&responses[last - 2]), ["0"]);
    assert_eq!(
        responses[last - 1]["Counts"],
        json!({"Visible": 99_999, "InFlight": 1, "Delayed": 0})
    );
}

#[test]
fn message_ids_count_accepted_messages_only() {
    let responses = run(&[
        create("q", json!({})),
        send("nope", "x"),
        send("q", ""),
        send("q", "a"),
    ]);
    assert_eq!(responses[3], json!({"ok": true, "MessageId": "m-1"}));
}

#[test]
fn the_binary_answers_over_stdin_and_stdout_and_exits_zero() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_miniq"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("the miniq binary starts");
    child
        .stdin
        .take()
        .expect("stdin is piped")
        .write_all(b"{\"Command\":\"Tick\",\"Seconds\":7}\n{\"Command\":\"Tick\",\"Seconds\":1}")
        .expect("the child reads stdin");
    let output = child.wait_with_output().expect("the child exits");
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).expect("responses are UTF-8"),
        "{\"ok\":true,\"Now\":7}\n{\"ok\":true,\"Now\":8}\n"
    );
}
