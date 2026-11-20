//! End-to-end scenarios: command lines in, response values out.

use std::io::Cursor;

use serde_json::{Value, json};

fn run_lines(lines: &[String]) -> Vec<Value> {
    let mut input = lines.join("\n");
    input.push('\n');
    let mut output = Vec::new();
    miniq::run(Cursor::new(input), &mut output).unwrap();
    String::from_utf8(output)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn session(commands: &[Value]) -> Vec<Value> {
    let lines: Vec<String> = commands.iter().map(Value::to_string).collect();
    run_lines(&lines)
}

fn ok() -> Value {
    json!({"ok": true})
}

fn code(response: &Value) -> &str {
    assert_eq!(response["ok"], json!(false), "{response}");
    response["error"]["code"].as_str().unwrap()
}

fn create_raw(name: &str, attributes: Value) -> Value {
    json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attributes})
}

/// Like `create_raw`, but names ending in `.fifo` also get `Fifo: true`.
fn create(name: &str, mut attributes: Value) -> Value {
    if name.ends_with(".fifo") {
        attributes["Fifo"] = json!(true);
    }
    create_raw(name, attributes)
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

fn delete(queue: &str, handle: &str) -> Value {
    json!({"Command": "DeleteMessage", "QueueName": queue, "ReceiptHandle": handle})
}

fn bodies(response: &Value) -> Vec<&str> {
    response["Messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["Body"].as_str().unwrap())
        .collect()
}

#[test]
fn every_line_gets_one_response_in_order() {
    let lines = [
        "".to_owned(),
        r#"{"Command": "Tick", "Seconds": 5}"#.to_owned(),
        "garbage".to_owned(),
        r#"{"Command": "Tick", "Seconds": 5}"#.to_owned(),
    ];
    let responses = run_lines(&lines);
    assert_eq!(responses.len(), 4);
    assert_eq!(code(&responses[0]), "InvalidRequest");
    assert_eq!(responses[1], json!({"ok": true, "Now": 5}));
    assert_eq!(code(&responses[2]), "InvalidRequest");
    assert_eq!(responses[3], json!({"ok": true, "Now": 10}));
}

#[test]
fn final_line_without_newline_counts() {
    let mut output = Vec::new();
    let input = Cursor::new(&b"{\"Command\": \"Tick\", \"Seconds\": 1}"[..]);
    miniq::run(input, &mut output).unwrap();
    assert_eq!(output, b"{\"ok\":true,\"Now\":1}\n");
}

#[test]
fn invalid_utf8_is_an_invalid_request_and_the_run_continues() {
    let mut output = Vec::new();
    let input =
        Cursor::new(&b"{\"Command\": \"\xff\"}\n{\"Command\": \"Tick\", \"Seconds\": 1}\n"[..]);
    miniq::run(input, &mut output).unwrap();
    let text = String::from_utf8(output).unwrap();
    let lines: Vec<Value> = text
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(code(&lines[0]), "InvalidRequest");
    assert_eq!(lines[1], json!({"ok": true, "Now": 1}));
}

#[test]
fn create_is_idempotent_only_for_equal_attributes() {
    let r = session(&[
        create("q", json!({"VisibilityTimeout": 10})),
        create("q", json!({"VisibilityTimeout": 10, "DelaySeconds": 0})),
        create("q", json!({})),
        create("q", json!({"VisibilityTimeout": 10, "DelaySeconds": null})),
    ]);
    assert_eq!(r[0], ok());
    assert_eq!(r[1], ok());
    assert_eq!(code(&r[2]), "QueueAlreadyExists");
    assert_eq!(r[3], ok());
}

#[test]
fn create_rule_order() {
    let r = session(&[
        create_raw("a.fifo", json!({})),
        create_raw("b", json!({"Fifo": true})),
        create_raw("c", json!({"ContentBasedDeduplication": true})),
        create_raw("d", json!({"DeadLetterQueue": "x"})),
        create_raw(
            "e",
            json!({"DeadLetterQueue": "missing", "MaxReceiveCount": 2}),
        ),
        create_raw("f.fifo", json!({"Fifo": true})),
        create_raw("std", json!({})),
        create_raw(
            "g.fifo",
            json!({"Fifo": true, "DeadLetterQueue": "std", "MaxReceiveCount": 2}),
        ),
        create_raw("h", json!({"DeadLetterQueue": "std", "MaxReceiveCount": 2})),
    ]);
    let codes: Vec<&str> = r[..8]
        .iter()
        .map(|v| {
            if v["ok"] == json!(true) {
                "ok"
            } else {
                code(v)
            }
        })
        .collect();
    assert_eq!(
        codes,
        [
            "QueueNameInvalid",
            "QueueNameInvalid",
            "InvalidParameterValue",
            "InvalidParameterValue",
            "QueueDoesNotExist",
            "ok",
            "ok",
            "InvalidParameterValue",
        ]
    );
    assert_eq!(r[8], ok());
}

#[test]
fn set_attributes_rules_and_effects() {
    let set = |queue: &str, attributes: Value| json!({"Command": "SetQueueAttributes", "QueueName": queue, "Attributes": attributes});
    let r = session(&[
        create("q", json!({})),
        create("dlq", json!({})),
        create("f.fifo", json!({"Fifo": true})),
        set("q", json!({"Fifo": false})),
        set("q", json!({"ContentBasedDeduplication": true})),
        set("q", json!({"MaxReceiveCount": 2})),
        set(
            "q",
            json!({"DeadLetterQueue": "nope", "MaxReceiveCount": 2}),
        ),
        set("q", json!({"DeadLetterQueue": "q", "MaxReceiveCount": 2})),
        set(
            "q",
            json!({"DeadLetterQueue": "f.fifo", "MaxReceiveCount": 2}),
        ),
        set(
            "q",
            json!({"DeadLetterQueue": "dlq", "MaxReceiveCount": 2, "DelaySeconds": 9}),
        ),
        set("q", json!({"VisibilityTimeout": 5})),
        counts("q"),
        json!({"Command": "SetQueueAttributes", "QueueName": "q"}),
    ]);
    let codes: Vec<&str> = r[3..9].iter().map(code).collect();
    assert_eq!(
        codes,
        [
            "UnsupportedOperation",
            "InvalidParameterValue",
            "InvalidParameterValue",
            "QueueDoesNotExist",
            "InvalidParameterValue",
            "InvalidParameterValue",
        ]
    );
    assert_eq!(r[9], ok());
    assert_eq!(r[10], ok());
    assert_eq!(
        r[11],
        json!({
            "ok": true,
            "Attributes": {
                "VisibilityTimeout": 5, "DelaySeconds": 9, "RetentionSeconds": 345600,
                "Fifo": false, "ContentBasedDeduplication": false,
                "DeadLetterQueue": "dlq", "MaxReceiveCount": 2
            },
            "Counts": {"Visible": 0, "InFlight": 0, "Delayed": 0}
        })
    );
    assert_eq!(code(&r[12]), "MissingParameter");
}

#[test]
fn unknown_queue_beats_rule_errors() {
    let r = session(&[
        json!({"Command": "SetQueueAttributes", "QueueName": "nope", "Attributes": {"Fifo": true}}),
        json!({"Command": "SendMessage", "QueueName": "nope", "Body": "x", "GroupId": "g"}),
        json!({"Command": "DeleteMessage", "QueueName": "nope", "ReceiptHandle": "junk"}),
        json!({"Command": "SendMessage", "QueueName": "bad name!", "Body": "x"}),
    ]);
    for response in &r {
        assert_eq!(code(response), "QueueDoesNotExist");
    }
}

#[test]
fn list_queues_is_sorted_by_bytes_and_filtered_by_prefix() {
    let mut commands: Vec<Value> = ["b", "a_b", "A", "a-b", "a.fifo", "a1", "ab"]
        .iter()
        .map(|name| create(name, json!({"Fifo": name.ends_with(".fifo")})))
        .collect();
    commands.push(json!({"Command": "ListQueues"}));
    commands.push(json!({"Command": "ListQueues", "Prefix": "a"}));
    commands.push(json!({"Command": "ListQueues", "Prefix": "zz"}));
    let r = session(&commands);
    assert_eq!(
        r[7],
        json!({"ok": true, "QueueNames": ["A", "a-b", "a.fifo", "a1", "a_b", "ab", "b"]})
    );
    assert_eq!(
        r[8],
        json!({"ok": true, "QueueNames": ["a-b", "a.fifo", "a1", "a_b", "ab"]})
    );
    assert_eq!(r[9], json!({"ok": true, "QueueNames": []}));
}

#[test]
fn standard_queue_lifecycle() {
    let r = session(&[
        create("q", json!({"VisibilityTimeout": 10})),
        send("q", "one"),
        send("q", "two"),
        json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 5}),
        counts("q"),
        tick(9),
        receive("q"),
        tick(1),
        receive("q"),
    ]);
    assert_eq!(r[1], json!({"ok": true, "MessageId": "m-1"}));
    assert_eq!(r[2], json!({"ok": true, "MessageId": "m-2"}));
    assert_eq!(
        r[3],
        json!({"ok": true, "Messages": [
            {"MessageId": "m-1", "ReceiptHandle": "r-1", "Body": "one", "ReceiveCount": 1, "SentAt": 0, "FirstReceivedAt": 0},
            {"MessageId": "m-2", "ReceiptHandle": "r-2", "Body": "two", "ReceiveCount": 1, "SentAt": 0, "FirstReceivedAt": 0},
        ]})
    );
    assert_eq!(
        r[4]["Counts"],
        json!({"Visible": 0, "InFlight": 2, "Delayed": 0})
    );
    assert_eq!(r[6], json!({"ok": true, "Messages": []}));
    assert_eq!(bodies(&r[8]), ["one"]);
    assert_eq!(r[8]["Messages"][0]["ReceiveCount"], json!(2));
    assert_eq!(r[8]["Messages"][0]["FirstReceivedAt"], json!(0));
    assert_eq!(r[8]["Messages"][0]["ReceiptHandle"], json!("r-3"));
}

#[test]
fn delay_applies_per_message_or_from_the_queue_at_send_time() {
    let r = session(&[
        create("q", json!({"DelaySeconds": 5})),
        send("q", "queue-delay"),
        json!({"Command": "SendMessage", "QueueName": "q", "Body": "own", "DelaySeconds": 0}),
        json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"DelaySeconds": 100}}),
        counts("q"),
        tick(5),
        counts("q"),
    ]);
    assert_eq!(
        r[4]["Counts"],
        json!({"Visible": 1, "InFlight": 0, "Delayed": 1})
    );
    assert_eq!(
        r[6]["Counts"],
        json!({"Visible": 2, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn retention_removes_messages_at_once_and_forever() {
    let r = session(&[
        create("q", json!({"RetentionSeconds": 100})),
        send("q", "old"),
        tick(60),
        send("q", "young"),
        tick(39),
        counts("q"),
        tick(1),
        counts("q"),
        json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"RetentionSeconds": 60}}),
        counts("q"),
        json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"RetentionSeconds": 1000}}),
        counts("q"),
    ]);
    assert_eq!(r[5]["Counts"]["Visible"], json!(2));
    assert_eq!(r[7]["Counts"]["Visible"], json!(1));
    assert_eq!(r[9]["Counts"]["Visible"], json!(1));
    assert_eq!(r[11]["Counts"]["Visible"], json!(1));
}

#[test]
fn expired_in_flight_message_makes_its_handle_stale() {
    let r = session(&[
        create(
            "q",
            json!({"RetentionSeconds": 60, "VisibilityTimeout": 1000}),
        ),
        send("q", "x"),
        receive("q"),
        tick(60),
        delete("q", "r-1"),
    ]);
    assert_eq!(code(&r[4]), "ReceiptHandleExpired");
}

#[test]
fn receipt_handle_errors() {
    let r = session(&[
        create("q", json!({"VisibilityTimeout": 10})),
        create("other", json!({})),
        send("q", "x"),
        receive("q"),
        delete("q", "r-01"),
        delete("q", "R-1"),
        delete("q", "r-1 "),
        delete("q", "r-2"),
        delete("other", "r-1"),
        json!({"Command": "ChangeMessageVisibility", "QueueName": "q", "ReceiptHandle": "r-1", "VisibilityTimeout": 50}),
        tick(40),
        receive("q"),
        tick(11),
        receive("q"),
        delete("q", "r-1"),
        delete("q", "r-2"),
    ]);
    let codes: Vec<&str> = [4, 5, 6, 7].iter().map(|&i| code(&r[i])).collect();
    assert_eq!(codes, ["ReceiptHandleInvalid"; 4]);
    assert_eq!(code(&r[8]), "ReceiptHandleExpired");
    assert_eq!(r[9], ok());
    assert_eq!(r[11], json!({"ok": true, "Messages": []}));
    assert_eq!(bodies(&r[13]), ["x"]);
    assert_eq!(code(&r[14]), "ReceiptHandleExpired");
    assert_eq!(r[15], ok());
}

#[test]
fn zero_visibility_makes_the_handle_unusable_and_change_to_zero_releases() {
    let r = session(&[
        create("q", json!({})),
        send("q", "x"),
        json!({"Command": "ReceiveMessage", "QueueName": "q", "VisibilityTimeout": 0}),
        delete("q", "r-1"),
        receive("q"),
        json!({"Command": "ChangeMessageVisibility", "QueueName": "q", "ReceiptHandle": "r-2", "VisibilityTimeout": 0}),
        counts("q"),
        delete("q", "r-2"),
    ]);
    assert_eq!(code(&r[3]), "ReceiptHandleExpired");
    assert_eq!(r[5], ok());
    assert_eq!(
        r[6]["Counts"],
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(code(&r[7]), "ReceiptHandleExpired");
}

#[test]
fn purge_empties_the_queue_but_not_fifo_state() {
    let fifo_send = |body: &str| json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": body, "GroupId": "g", "DeduplicationId": body});
    let r = session(&[
        create("f.fifo", json!({})),
        fifo_send("a"),
        json!({"Command": "PurgeQueue", "QueueName": "f.fifo"}),
        fifo_send("a"),
        fifo_send("b"),
        counts("f.fifo"),
    ]);
    assert_eq!(
        r[1],
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
    assert_eq!(
        r[3],
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
    assert_eq!(
        r[4],
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2})
    );
    assert_eq!(r[5]["Counts"]["Visible"], json!(1));
}

#[test]
fn send_rules_by_queue_type() {
    let r = session(&[
        create("s", json!({})),
        create("f.fifo", json!({})),
        create("c.fifo", json!({"ContentBasedDeduplication": true})),
        json!({"Command": "SendMessage", "QueueName": "s", "Body": "x", "GroupId": "g"}),
        json!({"Command": "SendMessage", "QueueName": "s", "Body": "x", "DeduplicationId": "d"}),
        json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "x", "GroupId": "g", "DeduplicationId": "d", "DelaySeconds": 1}),
        json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "x", "DeduplicationId": "d"}),
        json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "x", "GroupId": "g"}),
        json!({"Command": "SendMessage", "QueueName": "c.fifo", "Body": "x", "GroupId": "g"}),
        json!({"Command": "SendMessage", "QueueName": "s", "Body": ""}),
        json!({"Command": "SendMessage", "QueueName": "s"}),
    ]);
    let codes: Vec<&str> = r[3..8].iter().map(code).collect();
    assert_eq!(
        codes,
        [
            "UnsupportedOperation",
            "UnsupportedOperation",
            "UnsupportedOperation",
            "MissingParameter",
            "MissingParameter",
        ]
    );
    assert_eq!(
        r[8],
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
    assert_eq!(code(&r[9]), "InvalidParameterValue");
    assert_eq!(code(&r[10]), "MissingParameter");
}

#[test]
fn deduplication_window_is_fixed_and_survives_the_message() {
    let fifo_send = |body: &str, dedup: &str| json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": body, "GroupId": "g", "DeduplicationId": dedup});
    let r = session(&[
        create("f.fifo", json!({})),
        fifo_send("a", "d"),
        tick(299),
        fifo_send("dup", "d"),
        receive("f.fifo"),
        delete("f.fifo", "r-1"),
        fifo_send("again", "d"),
        tick(1),
        fifo_send("new", "d"),
        fifo_send("other-group-same-id", "d"),
    ]);
    assert_eq!(
        r[1],
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
    assert_eq!(r[3], r[1]);
    assert_eq!(r[6], r[1]);
    assert_eq!(
        r[8],
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2})
    );
    assert_eq!(r[9], r[8]);
}

#[test]
fn content_based_deduplication_uses_the_body() {
    let send_body = |body: &str| json!({"Command": "SendMessage", "QueueName": "c.fifo", "Body": body, "GroupId": "g"});
    let r = session(&[
        create("c.fifo", json!({"ContentBasedDeduplication": true})),
        send_body("same"),
        send_body("same"),
        send_body("different"),
    ]);
    assert_eq!(r[2], r[1]);
    assert_eq!(r[3]["SequenceNumber"], json!(2));
    assert_eq!(r[3]["MessageId"], json!("m-2"));
}

#[test]
fn fifo_delivers_one_message_per_group_and_keeps_group_order() {
    let send_to = |group: &str, body: &str| json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": body, "GroupId": group, "DeduplicationId": body});
    let r = session(&[
        create("f.fifo", json!({"VisibilityTimeout": 10})),
        send_to("a", "a1"),
        send_to("a", "a2"),
        send_to("b", "b1"),
        json!({"Command": "ReceiveMessage", "QueueName": "f.fifo", "MaxMessages": 10}),
        json!({"Command": "ReceiveMessage", "QueueName": "f.fifo", "MaxMessages": 10}),
        delete("f.fifo", "r-1"),
        json!({"Command": "ReceiveMessage", "QueueName": "f.fifo", "MaxMessages": 10}),
    ]);
    assert_eq!(bodies(&r[4]), ["a1", "b1"]);
    assert_eq!(r[4]["Messages"][0]["GroupId"], json!("a"));
    assert_eq!(r[4]["Messages"][0]["SequenceNumber"], json!(1));
    assert_eq!(r[5], json!({"ok": true, "Messages": []}));
    assert_eq!(bodies(&r[7]), ["a2"]);
}

#[test]
fn delayed_head_of_a_group_blocks_the_group() {
    let r = session(&[
        create("f.fifo", json!({"DelaySeconds": 5})),
        json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "x", "GroupId": "g", "DeduplicationId": "1"}),
        json!({"Command": "SetQueueAttributes", "QueueName": "f.fifo", "Attributes": {"DelaySeconds": 0}}),
        json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "y", "GroupId": "g", "DeduplicationId": "2"}),
        receive("f.fifo"),
        tick(5),
        receive("f.fifo"),
    ]);
    assert_eq!(r[4], json!({"ok": true, "Messages": []}));
    assert_eq!(bodies(&r[6]), ["x"]);
}

#[test]
fn dead_letter_move_resets_the_message_and_frees_its_group() {
    let r = session(&[
        create("dlq", json!({})),
        create(
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
    assert_eq!(bodies(&r[4]), ["poison"]);
    assert_eq!(bodies(&r[5]), ["poison"]);
    assert_eq!(r[5]["Messages"][0]["ReceiveCount"], json!(2));
    assert_eq!(bodies(&r[6]), ["fine"]);
    assert_eq!(r[7]["Counts"]["Visible"], json!(1));
    assert_eq!(r[8]["Counts"]["Visible"], json!(1));
    let moved = &r[9]["Messages"][0];
    assert_eq!(moved["MessageId"], json!("m-1"));
    assert_eq!(moved["ReceiveCount"], json!(1));
    assert_eq!(moved["SentAt"], json!(0));
    assert_eq!(moved["FirstReceivedAt"], json!(0));
}

#[test]
fn dead_letter_arrival_respects_the_target_retention_and_fifo_sequence() {
    let send_to = |queue: &str, body: &str| json!({"Command": "SendMessage", "QueueName": queue, "Body": body, "GroupId": "g", "DeduplicationId": body});
    let r = session(&[
        create("dlq.fifo", json!({"Fifo": true, "RetentionSeconds": 60})),
        create(
            "src.fifo",
            json!({"Fifo": true, "VisibilityTimeout": 0, "DeadLetterQueue": "dlq.fifo", "MaxReceiveCount": 1}),
        ),
        send_to("dlq.fifo", "native"),
        send_to("src.fifo", "old"),
        receive("src.fifo"),
        tick(70),
        send_to("src.fifo", "newer"),
        receive("src.fifo"),
        counts("dlq.fifo"),
        send_to("dlq.fifo", "after"),
    ]);
    assert_eq!(bodies(&r[7]), ["newer"]);
    assert_eq!(r[8]["Counts"]["Visible"], json!(0));
    assert_eq!(r[9]["SequenceNumber"], json!(3));
}

#[test]
fn deleting_a_queue_clears_dead_letter_references_and_forgets_its_handles() {
    let r = session(&[
        create("dlq", json!({})),
        create("q", json!({"DeadLetterQueue": "dlq", "MaxReceiveCount": 2})),
        create("dlq2", json!({})),
        send("dlq", "x"),
        receive("dlq"),
        json!({"Command": "DeleteQueue", "QueueName": "dlq"}),
        counts("q"),
        create("dlq", json!({})),
        delete("dlq", "r-1"),
        json!({"Command": "DeleteQueue", "QueueName": "dlq"}),
        json!({"Command": "DeleteQueue", "QueueName": "dlq"}),
    ]);
    assert_eq!(r[5], ok());
    assert!(r[6]["Attributes"].get("DeadLetterQueue").is_none());
    assert!(r[6]["Attributes"].get("MaxReceiveCount").is_none());
    assert_eq!(r[7], ok());
    assert_eq!(code(&r[8]), "ReceiptHandleExpired");
    assert_eq!(code(&r[10]), "QueueDoesNotExist");
}

#[test]
fn queue_and_message_limits() {
    let mut commands = Vec::new();
    for i in 0..1001 {
        commands.push(create(&format!("q{i}"), json!({})));
    }
    let r = session(&commands);
    assert!(r[..1000].iter().all(|v| *v == ok()));
    assert_eq!(code(&r[1000]), "LimitExceeded");

    let mut lines = vec![r#"{"Command":"CreateQueue","QueueName":"big"}"#.to_owned()];
    let line = r#"{"Command":"SendMessage","QueueName":"big","Body":"x"}"#;
    lines.extend(std::iter::repeat_n(line.to_owned(), 100_001));
    let r = run_lines(&lines);
    assert_eq!(r[100_000], json!({"ok": true, "MessageId": "m-100000"}));
    assert_eq!(code(&r[100_001]), "LimitExceeded");
}

#[test]
fn tick_reports_the_clock() {
    let r = session(&[tick(0), tick(7), tick(1_209_600)]);
    assert_eq!(r[0], json!({"ok": true, "Now": 0}));
    assert_eq!(r[2], json!({"ok": true, "Now": 1_209_607}));
}
