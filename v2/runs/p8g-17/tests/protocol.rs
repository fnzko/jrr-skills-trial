//! End-to-end behavior through the line protocol, one scenario per test.

use std::io::Cursor;

use serde_json::{Value, json};

/// Runs the lines through the service and returns the parsed response lines.
fn session(lines: &[Value]) -> Vec<Value> {
    let input: String = lines.iter().map(|line| format!("{line}\n")).collect();
    run_text(input.as_bytes())
}

fn run_text(input: &[u8]) -> Vec<Value> {
    let mut output = Vec::new();
    miniq::run(Cursor::new(input), &mut output).expect("in-memory io cannot fail");
    String::from_utf8(output)
        .expect("responses are UTF-8")
        .lines()
        .map(|line| serde_json::from_str(line).expect("each response is one JSON object"))
        .collect()
}

fn ok() -> Value {
    json!({"ok": true})
}

fn error_code(response: &Value) -> &str {
    assert_eq!(response["ok"], json!(false), "{response}");
    response["error"]["code"]
        .as_str()
        .expect("error has a code")
}

/// The error code of a failure, or `"ok"` for a success.
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

fn tick(seconds: u32) -> Value {
    json!({"Command": "Tick", "Seconds": seconds})
}

fn counts(queue: &str) -> Value {
    json!({"Command": "GetQueueAttributes", "QueueName": queue})
}

#[test]
fn every_line_gets_one_response_and_a_final_unterminated_line_counts() {
    let responses =
        run_text(b"\n{\"Command\":\"Tick\",\"Seconds\":1}\n\n{\"Command\":\"Tick\",\"Seconds\":2}");
    let codes: Vec<_> = responses.iter().map(|r| r["ok"].clone()).collect();
    assert_eq!(
        codes,
        [json!(false), json!(true), json!(false), json!(true)]
    );
    assert_eq!(responses[3], json!({"ok": true, "Now": 3}));
    assert_eq!(run_text(b""), Vec::<Value>::new());
}

#[test]
fn invalid_utf8_is_one_error_and_the_next_line_still_runs() {
    let responses = run_text(
        b"{\"Command\":\"Tick\",\"Seconds\":\"\xff\"}\n{\"Command\":\"Tick\",\"Seconds\":4}\n",
    );
    assert_eq!(error_code(&responses[0]), "InvalidRequest");
    assert_eq!(responses[1], json!({"ok": true, "Now": 4}));
}

#[test]
fn create_queue_is_idempotent_only_for_equal_attributes() {
    let responses = session(&[
        create("jobs", json!({})),
        create("jobs", json!({"VisibilityTimeout": 30, "Fifo": false})),
        create("jobs", json!({"VisibilityTimeout": 31})),
        create("jobs.fifo", json!({})),
        create("plain", json!({"Fifo": true})),
        create("plain", json!({"ContentBasedDeduplication": true})),
        create("half", json!({"MaxReceiveCount": 2})),
        create(
            "half",
            json!({"DeadLetterQueue": "ghost", "MaxReceiveCount": 2}),
        ),
        create("a b", json!(7)),
    ]);
    let codes: Vec<_> = responses.iter().map(|r| r["ok"] == json!(true)).collect();
    assert_eq!(
        codes,
        [true, true, false, false, false, false, false, false, false]
    );
    assert_eq!(error_code(&responses[2]), "QueueAlreadyExists");
    assert_eq!(error_code(&responses[3]), "QueueNameInvalid");
    assert_eq!(error_code(&responses[4]), "QueueNameInvalid");
    assert_eq!(error_code(&responses[5]), "InvalidParameterValue");
    assert_eq!(error_code(&responses[6]), "InvalidParameterValue");
    assert_eq!(error_code(&responses[7]), "QueueDoesNotExist");
    assert_eq!(error_code(&responses[8]), "QueueNameInvalid");
}

#[test]
fn dead_letter_queue_must_match_the_fifo_type() {
    let responses = session(&[
        create("dead", json!({})),
        create(
            "q.fifo",
            json!({"Fifo": true, "DeadLetterQueue": "dead", "MaxReceiveCount": 2}),
        ),
    ]);
    assert_eq!(responses[0], ok());
    assert_eq!(error_code(&responses[1]), "InvalidParameterValue");
}

#[test]
fn queue_limit_is_one_thousand() {
    let mut script: Vec<Value> = (0..1000)
        .map(|n| create(&format!("q{n}"), json!({})))
        .collect();
    script.push(create("one-too-many", json!({})));
    script.push(create("q0", json!({})));
    let responses = session(&script);
    assert!(responses[..1000].iter().all(|r| *r == ok()));
    assert_eq!(error_code(&responses[1000]), "LimitExceeded");
    assert_eq!(responses[1001], ok());
}

#[test]
fn attributes_report_defaults_counts_and_dead_letter_only_when_set() {
    let responses = session(&[
        create("dead", json!({})),
        create(
            "q",
            json!({"DeadLetterQueue": "dead", "MaxReceiveCount": 5, "DelaySeconds": 10}),
        ),
        send("q", "later"),
        counts("q"),
        counts("dead"),
    ]);
    assert_eq!(
        responses[3],
        json!({"ok": true,
            "Attributes": {"VisibilityTimeout": 30, "DelaySeconds": 10, "RetentionSeconds": 345600,
                "Fifo": false, "ContentBasedDeduplication": false,
                "DeadLetterQueue": "dead", "MaxReceiveCount": 5},
            "Counts": {"Visible": 0, "InFlight": 0, "Delayed": 1}})
    );
    assert_eq!(
        responses[4],
        json!({"ok": true,
            "Attributes": {"VisibilityTimeout": 30, "DelaySeconds": 0, "RetentionSeconds": 345600,
                "Fifo": false, "ContentBasedDeduplication": false},
            "Counts": {"Visible": 0, "InFlight": 0, "Delayed": 0}})
    );
}

#[test]
fn set_queue_attributes_applies_the_rules_in_order() {
    let set = |attributes: Value| json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": attributes});
    let responses = session(&[
        create("q", json!({})),
        create("other.fifo", json!({"Fifo": true})),
        set(json!({"Fifo": false})),
        set(json!({"ContentBasedDeduplication": true})),
        set(json!({"DeadLetterQueue": "q"})),
        set(json!({"DeadLetterQueue": "ghost", "MaxReceiveCount": 1})),
        set(json!({"DeadLetterQueue": "q", "MaxReceiveCount": 1})),
        set(json!({"DeadLetterQueue": "other.fifo", "MaxReceiveCount": 1})),
        set(json!({"VisibilityTimeout": 0, "Unknown": 1})),
        set(json!({"DelaySeconds": 7})),
        counts("q"),
    ]);
    let codes: Vec<_> = responses[2..9].iter().map(error_code).collect();
    assert_eq!(
        codes,
        [
            "UnsupportedOperation",
            "InvalidParameterValue",
            "InvalidParameterValue",
            "QueueDoesNotExist",
            "InvalidParameterValue",
            "InvalidParameterValue",
            "InvalidParameterValue",
        ]
    );
    assert_eq!(responses[9], ok());
    assert_eq!(responses[10]["Attributes"]["DelaySeconds"], json!(7));
    assert_eq!(responses[10]["Attributes"]["VisibilityTimeout"], json!(30));
}

#[test]
fn list_queues_sorts_by_bytes_and_filters_by_prefix() {
    let names = ["b", "a_1", "a-1", "a.fifo", "A1", "a1"];
    let mut script: Vec<Value> = names
        .iter()
        .map(|name| {
            let fifo = name.ends_with(".fifo");
            create(name, json!({"Fifo": fifo}))
        })
        .collect();
    script.push(json!({"Command": "ListQueues"}));
    script.push(json!({"Command": "ListQueues", "Prefix": "a"}));
    script.push(json!({"Command": "ListQueues", "Prefix": "zzz"}));
    let responses = session(&script);
    assert_eq!(
        responses[6],
        json!({"ok": true, "QueueNames": ["A1", "a-1", "a.fifo", "a1", "a_1", "b"]})
    );
    assert_eq!(
        responses[7],
        json!({"ok": true, "QueueNames": ["a-1", "a.fifo", "a1", "a_1"]})
    );
    assert_eq!(responses[8], json!({"ok": true, "QueueNames": []}));
}

#[test]
fn delete_queue_drops_dead_letter_links_and_a_new_queue_starts_fresh() {
    let responses = session(&[
        create("dead", json!({})),
        create(
            "q",
            json!({"DeadLetterQueue": "dead", "MaxReceiveCount": 1}),
        ),
        send("dead", "old"),
        json!({"Command": "DeleteQueue", "QueueName": "dead"}),
        counts("q"),
        create("dead", json!({})),
        counts("dead"),
        json!({"Command": "DeleteQueue", "QueueName": "nope"}),
    ]);
    assert_eq!(responses[3], ok());
    assert_eq!(responses[4]["Attributes"].get("DeadLetterQueue"), None);
    assert_eq!(responses[4]["Attributes"].get("MaxReceiveCount"), None);
    assert_eq!(
        responses[6]["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(error_code(&responses[7]), "QueueDoesNotExist");
}

#[test]
fn message_ids_count_over_all_queues_and_failed_sends_use_none() {
    let responses = session(&[
        create("a", json!({})),
        create("b", json!({})),
        send("a", "1"),
        send("nope", "x"),
        send("b", "2"),
        send("a", ""),
        send("a", "3"),
    ]);
    let ids: Vec<_> = [2, 4, 6]
        .iter()
        .map(|&i| responses[i]["MessageId"].clone())
        .collect();
    assert_eq!(ids, [json!("m-1"), json!("m-2"), json!("m-3")]);
}

#[test]
fn delay_hides_a_message_until_its_time() {
    let responses = session(&[
        create("q", json!({})),
        json!({"Command": "SendMessage", "QueueName": "q", "Body": "x", "DelaySeconds": 5}),
        receive("q"),
        tick(4),
        receive("q"),
        tick(1),
        receive("q"),
    ]);
    assert_eq!(responses[2]["Messages"], json!([]));
    assert_eq!(responses[4]["Messages"], json!([]));
    assert_eq!(
        responses[6]["Messages"],
        json!([{"MessageId": "m-1", "ReceiptHandle": "r-1", "Body": "x", "ReceiveCount": 1,
                "SentAt": 0, "FirstReceivedAt": 5}])
    );
}

#[test]
fn receive_returns_arrival_order_and_hides_messages_for_the_visibility_timeout() {
    let responses = session(&[
        create("q", json!({"VisibilityTimeout": 10})),
        send("q", "a"),
        send("q", "b"),
        send("q", "c"),
        json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 2}),
        receive("q"),
        receive("q"),
        tick(10),
        json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 10}),
    ]);
    let bodies = |response: &Value| -> Vec<Value> {
        response["Messages"]
            .as_array()
            .expect("array")
            .iter()
            .map(|m| m["Body"].clone())
            .collect()
    };
    assert_eq!(bodies(&responses[4]), [json!("a"), json!("b")]);
    assert_eq!(bodies(&responses[5]), [json!("c")]);
    assert_eq!(bodies(&responses[6]), Vec::<Value>::new());
    assert_eq!(bodies(&responses[8]), [json!("a"), json!("b"), json!("c")]);
    assert_eq!(responses[8]["Messages"][0]["ReceiveCount"], json!(2));
    assert_eq!(responses[8]["Messages"][0]["ReceiptHandle"], json!("r-4"));
    assert_eq!(responses[8]["Messages"][0]["FirstReceivedAt"], json!(0));
}

#[test]
fn zero_visibility_timeout_makes_the_handle_unusable() {
    let responses = session(&[
        create("q", json!({})),
        send("q", "a"),
        json!({"Command": "ReceiveMessage", "QueueName": "q", "VisibilityTimeout": 0}),
        json!({"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": "r-1"}),
        counts("q"),
    ]);
    assert_eq!(error_code(&responses[3]), "ReceiptHandleExpired");
    assert_eq!(responses[4]["Counts"]["Visible"], json!(1));
}

#[test]
fn receipt_handle_errors_distinguish_never_issued_from_expired() {
    let delete = |queue: &str, handle: &str| json!({"Command": "DeleteMessage", "QueueName": queue, "ReceiptHandle": handle});
    let responses = session(&[
        create("q", json!({"VisibilityTimeout": 5})),
        create("other", json!({})),
        send("q", "a"),
        receive("q"),
        delete("q", "r-01"),
        delete("q", "R-1"),
        delete("q", "r-1 "),
        delete("q", "r-2"),
        delete("other", "r-1"),
        tick(5),
        delete("q", "r-1"),
        receive("q"),
        delete("q", "r-1"),
        delete("q", "r-2"),
        delete("q", "r-2"),
        delete("ghost", "bogus"),
    ]);
    let codes: Vec<_> = responses[4..].iter().map(outcome).collect();
    assert_eq!(
        codes,
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
            "QueueDoesNotExist",
        ]
    );
}

#[test]
fn change_visibility_moves_the_end_of_the_timeout_without_touching_the_count() {
    let change = |handle: &str, seconds: u32| json!({"Command": "ChangeMessageVisibility", "QueueName": "q", "ReceiptHandle": handle, "VisibilityTimeout": seconds});
    let responses = session(&[
        create("q", json!({"VisibilityTimeout": 10})),
        send("q", "a"),
        receive("q"),
        tick(9),
        change("r-1", 5),
        tick(4),
        receive("q"),
        tick(1),
        receive("q"),
        change("r-2", 0),
        counts("q"),
        change("r-2", 3),
    ]);
    assert_eq!(responses[4], ok());
    assert_eq!(responses[6]["Messages"], json!([]));
    assert_eq!(responses[8]["Messages"][0]["ReceiveCount"], json!(2));
    assert_eq!(responses[9], ok());
    assert_eq!(
        responses[10]["Counts"],
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(error_code(&responses[11]), "ReceiptHandleExpired");
}

#[test]
fn retention_removes_messages_in_any_state_and_lowering_it_acts_at_once() {
    let set = |seconds: u32| json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"RetentionSeconds": seconds}});
    let responses = session(&[
        create("q", json!({"RetentionSeconds": 100})),
        send("q", "a"),
        tick(50),
        send("q", "b"),
        receive("q"),
        tick(20),
        counts("q"),
        set(60),
        counts("q"),
        set(1000),
        counts("q"),
        tick(980),
        counts("q"),
    ]);
    assert_eq!(
        responses[6]["Counts"],
        json!({"Visible": 1, "InFlight": 1, "Delayed": 0})
    );
    assert_eq!(
        responses[8]["Counts"],
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(
        responses[10]["Counts"],
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(
        responses[12]["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn standard_queue_rejects_fifo_parameters_and_fifo_rejects_per_message_delay() {
    let responses = session(&[
        create("s", json!({})),
        create("f.fifo", json!({"Fifo": true})),
        json!({"Command": "SendMessage", "QueueName": "s", "Body": "x", "GroupId": "g"}),
        json!({"Command": "SendMessage", "QueueName": "s", "Body": "x", "DeduplicationId": "d"}),
        json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "x", "GroupId": "g", "DeduplicationId": "d", "DelaySeconds": 0}),
        json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "x", "DeduplicationId": "d"}),
        json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "x", "GroupId": "g"}),
    ]);
    let codes: Vec<_> = responses[2..].iter().map(error_code).collect();
    assert_eq!(
        codes,
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
fn fifo_duplicates_return_the_original_and_the_window_is_not_extended() {
    let fifo = |body: &str, dedup: &str| json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": body, "GroupId": "g", "DeduplicationId": dedup});
    let responses = session(&[
        create("f.fifo", json!({"Fifo": true})),
        fifo("a", "d"),
        tick(299),
        fifo("b", "d"),
        tick(1),
        fifo("c", "d"),
        fifo("d", "other"),
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
fn deduplication_outlives_the_message_and_content_based_uses_the_body() {
    let responses = session(&[
        create(
            "f.fifo",
            json!({"Fifo": true, "ContentBasedDeduplication": true}),
        ),
        json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "same", "GroupId": "g"}),
        json!({"Command": "PurgeQueue", "QueueName": "f.fifo"}),
        json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "same", "GroupId": "h"}),
        json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "next", "GroupId": "h"}),
    ]);
    assert_eq!(responses[3], responses[1]);
    assert_eq!(
        responses[4],
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2})
    );
}

#[test]
fn fifo_delivers_one_message_per_group_and_keeps_groups_in_order() {
    let fifo = |body: &str, group: &str| json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": body, "GroupId": group, "DeduplicationId": body});
    let responses = session(&[
        create("f.fifo", json!({"Fifo": true, "VisibilityTimeout": 10})),
        fifo("a1", "a"),
        fifo("a2", "a"),
        fifo("b1", "b"),
        json!({"Command": "ReceiveMessage", "QueueName": "f.fifo", "MaxMessages": 10}),
        json!({"Command": "ReceiveMessage", "QueueName": "f.fifo", "MaxMessages": 10}),
        json!({"Command": "DeleteMessage", "QueueName": "f.fifo", "ReceiptHandle": "r-1"}),
        json!({"Command": "ReceiveMessage", "QueueName": "f.fifo", "MaxMessages": 10}),
    ]);
    let bodies = |response: &Value| -> Vec<Value> {
        response["Messages"]
            .as_array()
            .expect("array")
            .iter()
            .map(|m| m["Body"].clone())
            .collect()
    };
    assert_eq!(bodies(&responses[4]), [json!("a1"), json!("b1")]);
    assert_eq!(bodies(&responses[5]), Vec::<Value>::new());
    assert_eq!(bodies(&responses[7]), [json!("a2")]);
    assert_eq!(responses[4]["Messages"][1]["SequenceNumber"], json!(3));
    assert_eq!(responses[4]["Messages"][0]["GroupId"], json!("a"));
}

#[test]
fn messages_past_the_receive_limit_move_to_the_dead_letter_queue() {
    let responses = session(&[
        create("dead", json!({"RetentionSeconds": 60, "DelaySeconds": 50})),
        create(
            "q",
            json!({"DeadLetterQueue": "dead", "MaxReceiveCount": 2, "VisibilityTimeout": 0}),
        ),
        send("q", "poison"),
        send("q", "fine"),
        receive("q"),
        receive("q"),
        json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 5}),
        counts("dead"),
        receive("dead"),
    ]);
    assert_eq!(responses[4]["Messages"][0]["Body"], json!("poison"));
    assert_eq!(responses[5]["Messages"][0]["Body"], json!("poison"));
    // poison hit its limit and moved; fine is delivered in the same call
    assert_eq!(responses[6]["Messages"][0]["Body"], json!("fine"));
    assert_eq!(responses[6]["Messages"][0]["ReceiveCount"], json!(1));
    assert_eq!(
        responses[7]["Counts"],
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(
        responses[8]["Messages"],
        json!([{"MessageId": "m-1", "ReceiptHandle": "r-4", "Body": "poison", "ReceiveCount": 1,
                "SentAt": 0, "FirstReceivedAt": 0}])
    );
}

#[test]
fn dead_lettered_message_that_is_too_old_is_dropped_on_arrival() {
    let responses = session(&[
        create("dead", json!({"RetentionSeconds": 60})),
        create(
            "q",
            json!({"DeadLetterQueue": "dead", "MaxReceiveCount": 1, "VisibilityTimeout": 70}),
        ),
        send("q", "old"),
        receive("q"),
        tick(70),
        receive("q"),
        counts("dead"),
    ]);
    assert_eq!(responses[5]["Messages"], json!([]));
    assert_eq!(
        responses[6]["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn dead_letter_move_takes_the_next_fifo_sequence_number_and_ignores_dedup() {
    let responses = session(&[
        create("dead.fifo", json!({"Fifo": true})),
        create(
            "q.fifo",
            json!({"Fifo": true, "DeadLetterQueue": "dead.fifo", "MaxReceiveCount": 1, "VisibilityTimeout": 0}),
        ),
        json!({"Command": "SendMessage", "QueueName": "dead.fifo", "Body": "x", "GroupId": "g", "DeduplicationId": "d"}),
        json!({"Command": "SendMessage", "QueueName": "q.fifo", "Body": "y", "GroupId": "h", "DeduplicationId": "e"}),
        json!({"Command": "ReceiveMessage", "QueueName": "q.fifo"}),
        json!({"Command": "ReceiveMessage", "QueueName": "q.fifo"}),
        json!({"Command": "ReceiveMessage", "QueueName": "dead.fifo", "MaxMessages": 10}),
    ]);
    assert_eq!(responses[5]["Messages"], json!([]));
    assert_eq!(responses[6]["Messages"][1]["Body"], json!("y"));
    assert_eq!(responses[6]["Messages"][1]["SequenceNumber"], json!(2));
}

#[test]
fn purge_empties_the_queue_but_sequence_numbers_carry_on() {
    let fifo = |body: &str| json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": body, "GroupId": "g", "DeduplicationId": body});
    let responses = session(&[
        create("f.fifo", json!({"Fifo": true})),
        fifo("a"),
        receive("f.fifo"),
        json!({"Command": "PurgeQueue", "QueueName": "f.fifo"}),
        counts("f.fifo"),
        fifo("b"),
        json!({"Command": "DeleteMessage", "QueueName": "f.fifo", "ReceiptHandle": "r-1"}),
    ]);
    assert_eq!(
        responses[4]["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(responses[5]["SequenceNumber"], json!(2));
    assert_eq!(error_code(&responses[6]), "ReceiptHandleExpired");
}

#[test]
fn a_full_queue_refuses_more_and_filling_it_is_fast() {
    let mut input = String::from("{\"Command\":\"CreateQueue\",\"QueueName\":\"big\"}\n");
    for _ in 0..100_001 {
        input.push_str("{\"Command\":\"SendMessage\",\"QueueName\":\"big\",\"Body\":\"x\"}\n");
    }
    let responses = run_text(input.as_bytes());
    assert_eq!(responses.len(), 100_002);
    assert_eq!(responses[100_000]["MessageId"], json!("m-100000"));
    assert_eq!(error_code(&responses[100_001]), "LimitExceeded");
}

#[test]
fn check_order_puts_lookup_before_command_rules_and_values_before_lookup() {
    let responses = session(&[
        json!({"Command": "SendMessage", "QueueName": "ghost", "Body": "x", "GroupId": "g"}),
        json!({"Command": "SendMessage", "QueueName": "ghost", "Body": ""}),
        json!({"Command": "ReceiveMessage", "QueueName": "bad name!"}),
        json!({"Command": "SetQueueAttributes", "QueueName": "ghost", "Attributes": {"Fifo": true}}),
        json!({"Command": "SetQueueAttributes", "QueueName": "ghost", "Attributes": {"Nope": 1}}),
    ]);
    let codes: Vec<_> = responses.iter().map(error_code).collect();
    assert_eq!(
        codes,
        [
            "QueueDoesNotExist",
            "InvalidParameterValue",
            "QueueDoesNotExist",
            "QueueDoesNotExist",
            "InvalidParameterValue"
        ]
    );
}
