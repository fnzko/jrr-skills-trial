//! End-to-end tests: command lines in, response lines out.

use serde_json::{Value, json};

/// Feeds every line to a fresh broker and returns the responses.
fn run(lines: &[Value]) -> Vec<Value> {
    let input: String = lines.iter().map(|l| format!("{l}\n")).collect();
    run_raw(input.as_bytes())
}

fn run_raw(input: &[u8]) -> Vec<Value> {
    let mut output = Vec::new();
    miniq::run(input, &mut output).expect("in-memory io cannot fail");
    String::from_utf8(output)
        .expect("responses are UTF-8")
        .lines()
        .map(|l| serde_json::from_str(l).expect("each response is one JSON object"))
        .collect()
}

fn error_code(response: &Value) -> &str {
    assert_eq!(response["ok"], json!(false), "{response}");
    response["error"]["code"].as_str().expect("error code")
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

fn tick(seconds: u64) -> Value {
    json!({"Command": "Tick", "Seconds": seconds})
}

fn counts(queue: &str) -> Value {
    json!({"Command": "GetQueueAttributes", "QueueName": queue})
}

#[test]
fn every_line_gets_one_response_including_blank_and_unterminated_ones() {
    let responses =
        run_raw(b"\n{\"Command\":\"Tick\",\"Seconds\":5}\n\n{\"Command\":\"Tick\",\"Seconds\":1}");
    assert_eq!(
        responses
            .iter()
            .map(|r| r["ok"].clone())
            .collect::<Vec<_>>(),
        [json!(false), json!(true), json!(false), json!(true)]
    );
    assert_eq!(responses[3]["Now"], json!(6));
}

#[test]
fn malformed_lines_are_invalid_requests() {
    let inputs: [&[u8]; 8] = [
        b"not json",
        b"[1]",
        b"\"x\"",
        b"{}",
        b"{\"Command\": 3}",
        b"{\"Command\": \"Nope\"}",
        b"{\"Command\":\"Tick\",\"Seconds\":1} trailing",
        b"{\"Command\":\"Tick\",\"Seconds\":1,\"x\":\"\xff\"}",
    ];
    for input in inputs {
        let mut line = input.to_vec();
        line.push(b'\n');
        let responses = run_raw(&line);
        assert_eq!(error_code(&responses[0]), "InvalidRequest", "{input:?}");
    }
}

#[test]
fn parameter_checks_run_in_the_documented_order() {
    let cases: Vec<(Value, &str)> = vec![
        (json!({"Command": "SendMessage"}), "MissingParameter"),
        (
            json!({"Command": "SendMessage", "QueueName": 1}),
            "MissingParameter",
        ),
        (
            json!({"Command": "SendMessage", "QueueName": 1, "Body": "x"}),
            "InvalidParameterValue",
        ),
        (
            json!({"Command": "SendMessage", "QueueName": "none", "Body": ""}),
            "InvalidParameterValue",
        ),
        (
            json!({"Command": "SendMessage", "QueueName": "none", "Body": "x"}),
            "QueueDoesNotExist",
        ),
        (
            json!({"Command": "Tick", "Seconds": 1.5}),
            "InvalidParameterValue",
        ),
        (
            json!({"Command": "Tick", "Seconds": null}),
            "MissingParameter",
        ),
        (
            json!({"Command": "CreateQueue", "QueueName": "a b", "Attributes": 3}),
            "QueueNameInvalid",
        ),
        (
            create_with("q", json!({"Bogus": 1})),
            "InvalidParameterValue",
        ),
        (
            create_with("q", json!({"VisibilityTimeout": 43201})),
            "InvalidParameterValue",
        ),
        (create_with("q", json!({"Fifo": true})), "QueueNameInvalid"),
        (create_with("q.fifo", json!({})), "QueueNameInvalid"),
        (
            create_with("q", json!({"ContentBasedDeduplication": true})),
            "InvalidParameterValue",
        ),
        (
            create_with("q", json!({"MaxReceiveCount": 3})),
            "InvalidParameterValue",
        ),
        (
            create_with(
                "q",
                json!({"DeadLetterQueue": "nope", "MaxReceiveCount": 3}),
            ),
            "QueueDoesNotExist",
        ),
    ];
    for (command, expected) in cases {
        let responses = run(std::slice::from_ref(&command));
        assert_eq!(error_code(&responses[0]), expected, "{command}");
    }
}

#[test]
fn create_queue_is_idempotent_only_for_equal_attributes() {
    let responses = run(&[
        create_with("q", json!({"VisibilityTimeout": 10})),
        create_with("q", json!({"VisibilityTimeout": 10, "DelaySeconds": null})),
        create_with("q", json!({"VisibilityTimeout": 11})),
        create("q"),
    ]);
    assert_eq!(responses[0], json!({"ok": true}));
    assert_eq!(responses[1], json!({"ok": true}));
    assert_eq!(error_code(&responses[2]), "QueueAlreadyExists");
    assert_eq!(error_code(&responses[3]), "QueueAlreadyExists");
}

#[test]
fn get_queue_attributes_reports_defaults_and_dead_letter_configuration() {
    let responses = run(&[
        create("dead"),
        create_with(
            "q",
            json!({"DeadLetterQueue": "dead", "MaxReceiveCount": 2}),
        ),
        counts("q"),
        counts("dead"),
    ]);
    assert_eq!(
        responses[2],
        json!({
            "ok": true,
            "Attributes": {
                "VisibilityTimeout": 30, "DelaySeconds": 0, "RetentionSeconds": 345600,
                "Fifo": false, "ContentBasedDeduplication": false,
                "DeadLetterQueue": "dead", "MaxReceiveCount": 2,
            },
            "Counts": {"Visible": 0, "InFlight": 0, "Delayed": 0},
        })
    );
    assert_eq!(responses[3]["Attributes"].get("DeadLetterQueue"), None);
}

#[test]
fn list_queues_sorts_by_byte_value_and_filters_by_prefix() {
    let responses = run(&[
        create("b"),
        create("a_1"),
        create("a-1"),
        create("A1"),
        create_fifo("a1.fifo"),
        json!({"Command": "ListQueues"}),
        json!({"Command": "ListQueues", "Prefix": "a"}),
        json!({"Command": "ListQueues", "Prefix": ""}),
        json!({"Command": "ListQueues", "Prefix": "zz"}),
    ]);
    assert_eq!(
        responses[5]["QueueNames"],
        json!(["A1", "a-1", "a1.fifo", "a_1", "b"])
    );
    assert_eq!(responses[6]["QueueNames"], json!(["a-1", "a1.fifo", "a_1"]));
    assert_eq!(responses[7]["QueueNames"], responses[5]["QueueNames"]);
    assert_eq!(responses[8]["QueueNames"], json!([]));
}

#[test]
fn standard_queue_send_receive_delete_round_trip() {
    let responses = run(&[
        create("q"),
        send("q", "one"),
        send("q", "two"),
        tick(3),
        json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 5}),
        counts("q"),
        json!({"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": "r-1"}),
        json!({"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": "r-1"}),
        json!({"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": "r-3"}),
    ]);
    assert_eq!(responses[1], json!({"ok": true, "MessageId": "m-1"}));
    assert_eq!(responses[2], json!({"ok": true, "MessageId": "m-2"}));
    assert_eq!(
        responses[4],
        json!({"ok": true, "Messages": [
            {"MessageId": "m-1", "ReceiptHandle": "r-1", "Body": "one", "ReceiveCount": 1, "SentAt": 0, "FirstReceivedAt": 3},
            {"MessageId": "m-2", "ReceiptHandle": "r-2", "Body": "two", "ReceiveCount": 1, "SentAt": 0, "FirstReceivedAt": 3},
        ]})
    );
    assert_eq!(
        responses[5]["Counts"],
        json!({"Visible": 0, "InFlight": 2, "Delayed": 0})
    );
    assert_eq!(responses[6], json!({"ok": true}));
    assert_eq!(error_code(&responses[7]), "ReceiptHandleExpired");
    assert_eq!(error_code(&responses[8]), "ReceiptHandleInvalid");
}

#[test]
fn visibility_timeout_ends_and_message_keeps_its_place() {
    let responses = run(&[
        create_with("q", json!({"VisibilityTimeout": 10})),
        send("q", "a"),
        send("q", "b"),
        receive("q"),
        tick(9),
        receive("q"),
        tick(1),
        receive("q"),
    ]);
    assert_eq!(responses[3]["Messages"][0]["Body"], json!("a"));
    assert_eq!(responses[5]["Messages"][0]["Body"], json!("b"));
    let third = &responses[7]["Messages"][0];
    assert_eq!(third["Body"], json!("a"));
    assert_eq!(third["ReceiveCount"], json!(2));
    assert_eq!(third["FirstReceivedAt"], json!(0));
}

#[test]
fn zero_visibility_timeout_makes_the_handle_unusable() {
    let responses = run(&[
        create("q"),
        send("q", "a"),
        json!({"Command": "ReceiveMessage", "QueueName": "q", "VisibilityTimeout": 0}),
        json!({"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": "r-1"}),
    ]);
    assert_eq!(responses[2]["Messages"][0]["ReceiptHandle"], json!("r-1"));
    assert_eq!(error_code(&responses[3]), "ReceiptHandleExpired");
}

#[test]
fn change_message_visibility_replaces_the_end_of_the_timeout() {
    let responses = run(&[
        create("q"),
        send("q", "a"),
        receive("q"),
        json!({"Command": "ChangeMessageVisibility", "QueueName": "q", "ReceiptHandle": "r-1", "VisibilityTimeout": 100}),
        tick(99),
        counts("q"),
        tick(1),
        counts("q"),
        json!({"Command": "ChangeMessageVisibility", "QueueName": "q", "ReceiptHandle": "r-1", "VisibilityTimeout": 5}),
    ]);
    assert_eq!(responses[3], json!({"ok": true}));
    assert_eq!(responses[5]["Counts"]["InFlight"], json!(1));
    assert_eq!(responses[7]["Counts"]["Visible"], json!(1));
    assert_eq!(error_code(&responses[8]), "ReceiptHandleExpired");
}

#[test]
fn handles_are_compared_as_exact_strings_and_stay_with_their_queue() {
    let responses = run(&[
        create("q"),
        create("other"),
        send("q", "a"),
        receive("q"),
        json!({"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": "r-01"}),
        json!({"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": "R-1"}),
        json!({"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": "r-1 "}),
        json!({"Command": "DeleteMessage", "QueueName": "other", "ReceiptHandle": "r-1"}),
        json!({"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": 1}),
    ]);
    let got: Vec<&str> = responses[4..].iter().map(error_code).collect();
    assert_eq!(
        got,
        [
            "ReceiptHandleInvalid",
            "ReceiptHandleInvalid",
            "ReceiptHandleInvalid",
            "ReceiptHandleExpired",
            "InvalidParameterValue"
        ]
    );
}

#[test]
fn delay_applies_from_the_send_and_a_queue_default_is_read_at_send_time() {
    let responses = run(&[
        create_with("q", json!({"DelaySeconds": 5})),
        send("q", "queue-delay"),
        json!({"Command": "SendMessage", "QueueName": "q", "Body": "own", "DelaySeconds": 0}),
        json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"DelaySeconds": 100}}),
        counts("q"),
        tick(5),
        counts("q"),
    ]);
    assert_eq!(
        responses[4]["Counts"],
        json!({"Visible": 1, "InFlight": 0, "Delayed": 1})
    );
    assert_eq!(
        responses[6]["Counts"],
        json!({"Visible": 2, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn retention_removes_messages_and_lowering_it_applies_at_once() {
    let responses = run(&[
        create("q"),
        send("q", "a"),
        tick(100),
        send("q", "b"),
        json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"RetentionSeconds": 60}}),
        counts("q"),
        json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"RetentionSeconds": 1000}}),
        tick(59),
        counts("q"),
        tick(1),
        counts("q"),
    ]);
    assert_eq!(responses[5]["Counts"]["Visible"], json!(1));
    assert_eq!(responses[8]["Counts"]["Visible"], json!(1));
    assert_eq!(responses[10]["Counts"]["Visible"], json!(1));
}

#[test]
fn raising_retention_does_not_bring_back_expired_messages() {
    let responses = run(&[
        create_with("q", json!({"RetentionSeconds": 60})),
        send("q", "a"),
        tick(60),
        json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"RetentionSeconds": 1000}}),
        counts("q"),
    ]);
    assert_eq!(
        responses[4]["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

fn fifo_send(queue: &str, body: &str, group: &str, dedup: &str) -> Value {
    json!({"Command": "SendMessage", "QueueName": queue, "Body": body, "GroupId": group, "DeduplicationId": dedup})
}

fn create_fifo(name: &str) -> Value {
    create_with(name, json!({"Fifo": true}))
}

#[test]
fn send_restrictions_by_queue_type() {
    let responses = run(&[
        create("q"),
        create_fifo("f.fifo"),
        json!({"Command": "SendMessage", "QueueName": "q", "Body": "x", "GroupId": "g"}),
        json!({"Command": "SendMessage", "QueueName": "q", "Body": "x", "DeduplicationId": "d"}),
        json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "x", "DelaySeconds": 1}),
        send("f.fifo", "x"),
        json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "x", "GroupId": "g"}),
    ]);
    let got: Vec<&str> = responses[2..].iter().map(error_code).collect();
    assert_eq!(
        got,
        [
            "UnsupportedOperation",
            "UnsupportedOperation",
            "UnsupportedOperation",
            "MissingParameter",
            "MissingParameter"
        ]
    );
}

#[test]
fn duplicate_sends_return_the_original_and_use_no_ids() {
    let responses = run(&[
        create_fifo("f.fifo"),
        fifo_send("f.fifo", "a", "g", "d1"),
        tick(299),
        fifo_send("f.fifo", "other body", "other-group", "d1"),
        tick(1),
        fifo_send("f.fifo", "again", "g", "d1"),
        fifo_send("f.fifo", "next", "g", "d2"),
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
fn content_based_deduplication_uses_the_body_and_dedup_outlives_the_message() {
    let responses = run(&[
        create_with(
            "f.fifo",
            json!({"Fifo": true, "ContentBasedDeduplication": true}),
        ),
        json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "same", "GroupId": "g"}),
        json!({"Command": "PurgeQueue", "QueueName": "f.fifo"}),
        json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "same", "GroupId": "g"}),
        json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "diff", "GroupId": "g"}),
        counts("f.fifo"),
    ]);
    assert_eq!(responses[3], responses[1]);
    assert_eq!(responses[4]["SequenceNumber"], json!(2));
    assert_eq!(responses[5]["Counts"]["Visible"], json!(1));
}

#[test]
fn fifo_delivers_one_message_per_group_and_blocks_behind_unfinished_ones() {
    let responses = run(&[
        create_fifo("f.fifo"),
        fifo_send("f.fifo", "a1", "a", "1"),
        fifo_send("f.fifo", "a2", "a", "2"),
        fifo_send("f.fifo", "b1", "b", "3"),
        json!({"Command": "ReceiveMessage", "QueueName": "f.fifo", "MaxMessages": 10}),
        json!({"Command": "ReceiveMessage", "QueueName": "f.fifo", "MaxMessages": 10}),
        json!({"Command": "DeleteMessage", "QueueName": "f.fifo", "ReceiptHandle": "r-1"}),
        json!({"Command": "ReceiveMessage", "QueueName": "f.fifo", "MaxMessages": 10}),
    ]);
    let bodies = |r: &Value| -> Vec<Value> {
        r["Messages"]
            .as_array()
            .expect("messages")
            .iter()
            .map(|m| m["Body"].clone())
            .collect()
    };
    assert_eq!(bodies(&responses[4]), [json!("a1"), json!("b1")]);
    assert_eq!(responses[4]["Messages"][1]["GroupId"], json!("b"));
    assert_eq!(responses[4]["Messages"][1]["SequenceNumber"], json!(3));
    assert_eq!(bodies(&responses[5]), Vec::<Value>::new());
    assert_eq!(bodies(&responses[7]), [json!("a2")]);
}

#[test]
fn messages_move_to_the_dead_letter_queue_after_max_receive_count() {
    let responses = run(&[
        create_with("dead", json!({"RetentionSeconds": 100})),
        create_with(
            "q",
            json!({"DeadLetterQueue": "dead", "MaxReceiveCount": 2, "VisibilityTimeout": 0}),
        ),
        send("q", "poison"),
        receive("q"),
        receive("q"),
        receive("q"),
        counts("dead"),
        receive("dead"),
        json!({"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": "r-2"}),
    ]);
    assert_eq!(responses[3]["Messages"][0]["ReceiveCount"], json!(1));
    assert_eq!(responses[4]["Messages"][0]["ReceiveCount"], json!(2));
    assert_eq!(responses[5]["Messages"], json!([]));
    assert_eq!(responses[6]["Counts"]["Visible"], json!(1));
    let moved = &responses[7]["Messages"][0];
    assert_eq!(moved["MessageId"], json!("m-1"));
    assert_eq!(moved["ReceiveCount"], json!(1));
    assert_eq!(moved["ReceiptHandle"], json!("r-3"));
    assert_eq!(error_code(&responses[8]), "ReceiptHandleExpired");
}

#[test]
fn dead_letter_arrival_that_is_too_old_is_dropped_and_fifo_takes_a_sequence_number() {
    let responses = run(&[
        create_with("dead.fifo", json!({"Fifo": true, "RetentionSeconds": 60})),
        create_with(
            "q.fifo",
            json!({"Fifo": true, "DeadLetterQueue": "dead.fifo", "MaxReceiveCount": 1, "VisibilityTimeout": 0}),
        ),
        fifo_send("q.fifo", "old", "g", "1"),
        receive("q.fifo"),
        tick(60),
        receive("q.fifo"),
        counts("dead.fifo"),
        fifo_send("dead.fifo", "fresh", "g", "2"),
    ]);
    assert_eq!(responses[5]["Messages"], json!([]));
    assert_eq!(responses[6]["Counts"]["Visible"], json!(0));
    assert_eq!(responses[7]["SequenceNumber"], json!(2));
}

#[test]
fn dead_letter_queue_rules_on_set_and_delete() {
    let responses = run(&[
        create("dead"),
        create_fifo("f.fifo"),
        create("q"),
        json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"DeadLetterQueue": "q", "MaxReceiveCount": 1}}),
        json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"DeadLetterQueue": "f.fifo", "MaxReceiveCount": 1}}),
        json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"DeadLetterQueue": "gone", "MaxReceiveCount": 1}}),
        json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"MaxReceiveCount": 1}}),
        json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"Fifo": false}}),
        json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"ContentBasedDeduplication": true}}),
        json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"DeadLetterQueue": "dead", "MaxReceiveCount": 5}}),
        json!({"Command": "DeleteQueue", "QueueName": "dead"}),
        counts("q"),
        json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {}}),
    ]);
    let got: Vec<&str> = responses[3..9].iter().map(error_code).collect();
    assert_eq!(
        got,
        [
            "InvalidParameterValue",
            "InvalidParameterValue",
            "QueueDoesNotExist",
            "InvalidParameterValue",
            "UnsupportedOperation",
            "InvalidParameterValue"
        ]
    );
    assert_eq!(responses[9], json!({"ok": true}));
    assert_eq!(responses[11]["Attributes"].get("DeadLetterQueue"), None);
    assert_eq!(responses[11]["Attributes"].get("MaxReceiveCount"), None);
    assert_eq!(responses[12], json!({"ok": true}));
}

#[test]
fn a_recreated_queue_shares_nothing_with_the_deleted_one() {
    let responses = run(&[
        create("q"),
        send("q", "a"),
        receive("q"),
        json!({"Command": "DeleteQueue", "QueueName": "q"}),
        create("q"),
        json!({"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": "r-1"}),
        counts("q"),
        json!({"Command": "DeleteQueue", "QueueName": "gone"}),
    ]);
    assert_eq!(error_code(&responses[5]), "ReceiptHandleExpired");
    assert_eq!(responses[6]["Counts"]["Visible"], json!(0));
    assert_eq!(error_code(&responses[7]), "QueueDoesNotExist");
}

#[test]
fn queue_limit_is_one_thousand() {
    let mut lines: Vec<Value> = (0..1000).map(|i| create(&format!("q{i}"))).collect();
    lines.push(create("one-too-many"));
    lines.push(create("q0"));
    let responses = run(&lines);
    assert_eq!(
        responses[..1000]
            .iter()
            .filter(|r| r["ok"] == json!(true))
            .count(),
        1000
    );
    assert_eq!(error_code(&responses[1000]), "LimitExceeded");
    assert_eq!(responses[1001], json!({"ok": true}));
}

#[test]
fn message_limit_is_one_hundred_thousand_and_filling_is_fast() {
    let mut lines = vec![create("q")];
    lines.extend((0..100_001).map(|_| send("q", "x")));
    let responses = run(&lines);
    assert_eq!(responses[100_000]["MessageId"], json!("m-100000"));
    assert_eq!(error_code(&responses[100_001]), "LimitExceeded");
}
