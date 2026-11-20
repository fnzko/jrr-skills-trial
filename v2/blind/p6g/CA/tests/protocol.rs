use std::io::Cursor;

use serde_json::{Value, json};

/// Runs `input` through miniq and returns one parsed response per line.
fn session(input: &str) -> Vec<Value> {
    let mut output = Vec::new();
    miniq::run(Cursor::new(input.as_bytes()), &mut output).expect("in-memory I/O does not fail");
    String::from_utf8(output)
        .expect("responses are UTF-8")
        .lines()
        .map(|line| serde_json::from_str(line).expect("every response line is JSON"))
        .collect()
}

/// Sends each command (as a JSON value) and returns the responses.
fn commands(cmds: &[Value]) -> Vec<Value> {
    let input: String = cmds.iter().map(|c| format!("{c}\n")).collect();
    session(&input)
}

fn last(cmds: &[Value]) -> Value {
    commands(cmds).pop().expect("at least one command was sent")
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

fn create(name: &str) -> Value {
    json!({"Command": "CreateQueue", "QueueName": name})
}

fn create_with(name: &str, attributes: Value) -> Value {
    json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attributes})
}

fn send(queue: &str, body: &str) -> Value {
    json!({"Command": "SendMessage", "QueueName": queue, "Body": body})
}

fn send_fifo(queue: &str, body: &str, group: &str, dedup: &str) -> Value {
    json!({"Command": "SendMessage", "QueueName": queue, "Body": body, "GroupId": group, "DeduplicationId": dedup})
}

fn receive(queue: &str) -> Value {
    json!({"Command": "ReceiveMessage", "QueueName": queue})
}

fn receive_n(queue: &str, max: u32) -> Value {
    json!({"Command": "ReceiveMessage", "QueueName": queue, "MaxMessages": max})
}

fn tick(seconds: u64) -> Value {
    json!({"Command": "Tick", "Seconds": seconds})
}

fn counts(queue: &str) -> Value {
    json!({"Command": "GetQueueAttributes", "QueueName": queue})
}

fn bodies(response: &Value) -> Vec<&str> {
    response["Messages"]
        .as_array()
        .expect("a Messages array")
        .iter()
        .map(|m| m["Body"].as_str().expect("a body"))
        .collect()
}

#[test]
fn every_line_gets_one_response_including_blank_and_unterminated_ones() {
    let responses = session(
        "\n{\"Command\": \"Tick\", \"Seconds\": 5}\nnot json\n{\"Command\": \"Tick\", \"Seconds\": 1}",
    );
    let codes: Vec<_> = responses.iter().map(|r| r["ok"].clone()).collect();
    assert_eq!(
        codes,
        [json!(false), json!(true), json!(false), json!(true)]
    );
    assert_eq!(responses[3], json!({"ok": true, "Now": 6}));
    assert_eq!(session("").len(), 0);
}

#[test]
fn error_responses_have_a_code_and_message() {
    let response = session("[1]\n").remove(0);
    assert_eq!(response["ok"], json!(false));
    assert_eq!(response["error"]["code"], json!("InvalidRequest"));
    assert!(response["error"]["message"].is_string());
}

#[test]
fn invalid_utf8_is_answered_and_processing_continues() {
    let mut output = Vec::new();
    miniq::run(
        Cursor::new(&b"\xff\xfe\n{\"Command\": \"Tick\", \"Seconds\": 0}\n"[..]),
        &mut output,
    )
    .unwrap();
    let lines: Vec<Value> = String::from_utf8(output)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(error_code(&lines[0]), "InvalidRequest");
    assert_eq!(lines[1], json!({"ok": true, "Now": 0}));
}

#[test]
fn create_queue_is_idempotent_for_equal_attributes_only() {
    let r = commands(&[
        create("a"),
        create_with("a", json!({"VisibilityTimeout": 30, "DelaySeconds": null})),
        create_with("a", json!({"VisibilityTimeout": 31})),
    ]);
    assert_eq!(r[0], ok());
    assert_eq!(r[1], ok());
    assert_eq!(error_code(&r[2]), "QueueAlreadyExists");
}

#[test]
fn create_queue_applies_the_rules_in_order() {
    let cases = [
        (create("a.fifo"), "QueueNameInvalid"),
        (create_with("a", json!({"Fifo": true})), "QueueNameInvalid"),
        (create_with("a b", json!({"Bogus": 1})), "QueueNameInvalid"),
        (
            create_with("a", json!({"ContentBasedDeduplication": true})),
            "InvalidParameterValue",
        ),
        (
            create_with("a", json!({"DeadLetterQueue": "x"})),
            "InvalidParameterValue",
        ),
        (
            create_with("a", json!({"DeadLetterQueue": "x", "MaxReceiveCount": 0})),
            "InvalidParameterValue",
        ),
        (
            create_with("a", json!({"DeadLetterQueue": "x", "MaxReceiveCount": 2})),
            "QueueDoesNotExist",
        ),
        (
            create_with("a", json!({"RetentionSeconds": 59})),
            "InvalidParameterValue",
        ),
    ];
    for (command, code) in cases {
        assert_eq!(
            error_code(&last(std::slice::from_ref(&command))),
            code,
            "{command}"
        );
    }
    let r = last(&[
        create("std"),
        create_with(
            "b.fifo",
            json!({"Fifo": true, "DeadLetterQueue": "std", "MaxReceiveCount": 2}),
        ),
    ]);
    assert_eq!(error_code(&r), "InvalidParameterValue");
}

#[test]
fn queue_limit_is_one_thousand() {
    let mut cmds: Vec<Value> = (0..1000).map(|i| create(&format!("q{i}"))).collect();
    cmds.push(create("q0"));
    cmds.push(create("extra"));
    let r = commands(&cmds);
    assert!(r[..1001].iter().all(|x| *x == ok()));
    assert_eq!(error_code(&r[1001]), "LimitExceeded");
}

#[test]
fn get_queue_attributes_reports_defaults_and_counts() {
    let r = commands(&[create("a"), send("a", "x"), counts("a")]);
    assert_eq!(
        r[2],
        json!({
            "ok": true,
            "Attributes": {
                "VisibilityTimeout": 30, "DelaySeconds": 0, "RetentionSeconds": 345600,
                "Fifo": false, "ContentBasedDeduplication": false
            },
            "Counts": {"Visible": 1, "InFlight": 0, "Delayed": 0}
        })
    );
}

#[test]
fn get_queue_attributes_shows_dead_letter_configuration_when_set() {
    let r = commands(&[
        create("dead"),
        create_with(
            "a",
            json!({"DeadLetterQueue": "dead", "MaxReceiveCount": 3}),
        ),
        counts("a"),
    ]);
    assert_eq!(r[2]["Attributes"]["DeadLetterQueue"], json!("dead"));
    assert_eq!(r[2]["Attributes"]["MaxReceiveCount"], json!(3));
}

#[test]
fn list_queues_filters_by_prefix_in_byte_order() {
    let names = ["b", "a_", "a-", "A", "a.fifo", "a1", "ab"];
    let mut cmds: Vec<Value> = names
        .iter()
        .map(|n| {
            if n.ends_with(".fifo") {
                create_with(n, json!({"Fifo": true}))
            } else {
                create(n)
            }
        })
        .collect();
    cmds.push(json!({"Command": "ListQueues"}));
    cmds.push(json!({"Command": "ListQueues", "Prefix": "a"}));
    cmds.push(json!({"Command": "ListQueues", "Prefix": "zzz"}));
    let r = commands(&cmds);
    let n = names.len();
    assert_eq!(
        r[n]["QueueNames"],
        json!(["A", "a-", "a.fifo", "a1", "a_", "ab", "b"])
    );
    assert_eq!(
        r[n + 1]["QueueNames"],
        json!(["a-", "a.fifo", "a1", "a_", "ab"])
    );
    assert_eq!(r[n + 2], json!({"ok": true, "QueueNames": []}));
}

#[test]
fn message_and_receipt_ids_count_across_queues() {
    let r = commands(&[
        create("a"),
        create("b"),
        send("a", "1"),
        send("b", "2"),
        receive("b"),
        receive("a"),
    ]);
    assert_eq!(r[2], json!({"ok": true, "MessageId": "m-1"}));
    assert_eq!(r[3], json!({"ok": true, "MessageId": "m-2"}));
    assert_eq!(r[4]["Messages"][0]["ReceiptHandle"], json!("r-1"));
    assert_eq!(r[5]["Messages"][0]["ReceiptHandle"], json!("r-2"));
}

#[test]
fn receive_reports_the_message_and_hides_it_for_the_visibility_timeout() {
    let r = commands(&[
        create("a"),
        send("a", "hello"),
        tick(10),
        receive("a"),
        receive("a"),
        tick(29),
        receive("a"),
        tick(1),
        receive("a"),
    ]);
    assert_eq!(
        r[3]["Messages"][0],
        json!({
            "MessageId": "m-1", "ReceiptHandle": "r-1", "Body": "hello",
            "ReceiveCount": 1, "SentAt": 0, "FirstReceivedAt": 10
        })
    );
    assert_eq!(r[4], json!({"ok": true, "Messages": []}));
    assert_eq!(r[6], json!({"ok": true, "Messages": []}));
    assert_eq!(r[8]["Messages"][0]["ReceiveCount"], json!(2));
    assert_eq!(r[8]["Messages"][0]["FirstReceivedAt"], json!(10));
    assert_eq!(r[8]["Messages"][0]["ReceiptHandle"], json!("r-2"));
}

#[test]
fn receive_honours_max_messages_and_keeps_arrival_order() {
    let r = commands(&[
        create("a"),
        send("a", "1"),
        send("a", "2"),
        send("a", "3"),
        receive_n("a", 2),
        receive_n("a", 10),
    ]);
    assert_eq!(bodies(&r[4]), ["1", "2"]);
    assert_eq!(bodies(&r[5]), ["3"]);
}

#[test]
fn delayed_messages_become_visible_when_the_delay_ends() {
    let r = commands(&[
        create_with("a", json!({"DelaySeconds": 10})),
        send("a", "queue-delay"),
        json!({"Command": "SendMessage", "QueueName": "a", "Body": "own-delay", "DelaySeconds": 0}),
        counts("a"),
        tick(10),
        counts("a"),
    ]);
    assert_eq!(
        r[3]["Counts"],
        json!({"Visible": 1, "InFlight": 0, "Delayed": 1})
    );
    assert_eq!(
        r[5]["Counts"],
        json!({"Visible": 2, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn zero_visibility_timeout_makes_the_handle_unusable() {
    let r = commands(&[
        create("a"),
        send("a", "x"),
        json!({"Command": "ReceiveMessage", "QueueName": "a", "VisibilityTimeout": 0}),
        json!({"Command": "DeleteMessage", "QueueName": "a", "ReceiptHandle": "r-1"}),
    ]);
    assert_eq!(error_code(&r[3]), "ReceiptHandleExpired");
}

#[test]
fn delete_message_removes_it_and_the_handle_then_expires() {
    let delete = json!({"Command": "DeleteMessage", "QueueName": "a", "ReceiptHandle": "r-1"});
    let r = commands(&[
        create("a"),
        send("a", "x"),
        receive("a"),
        delete.clone(),
        delete,
        counts("a"),
    ]);
    assert_eq!(r[3], ok());
    assert_eq!(error_code(&r[4]), "ReceiptHandleExpired");
    assert_eq!(
        r[5]["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn receipt_handle_errors_follow_the_spec() {
    let with = |handle: &str, queue: &str| json!({"Command": "DeleteMessage", "QueueName": queue, "ReceiptHandle": handle});
    let r = commands(&[
        create("a"),
        create("b"),
        send("a", "x"),
        receive("a"),
        with("r-01", "a"),
        with("R-1", "a"),
        with("r-1 ", "a"),
        with("r-2", "a"),
        with("r-1", "b"),
        with("r-1", "missing"),
        with("bad", "missing"),
    ]);
    let codes: Vec<_> = r[4..].iter().map(error_code).collect();
    assert_eq!(
        codes,
        [
            "ReceiptHandleInvalid",
            "ReceiptHandleInvalid",
            "ReceiptHandleInvalid",
            "ReceiptHandleInvalid",
            "ReceiptHandleExpired",
            "QueueDoesNotExist",
            "QueueDoesNotExist"
        ]
    );
}

#[test]
fn an_old_handle_expires_when_the_message_is_delivered_again() {
    let delete =
        |h: &str| json!({"Command": "DeleteMessage", "QueueName": "a", "ReceiptHandle": h});
    let r = commands(&[
        create("a"),
        send("a", "x"),
        json!({"Command": "ReceiveMessage", "QueueName": "a", "VisibilityTimeout": 5}),
        tick(5),
        receive("a"),
        delete("r-1"),
        delete("r-2"),
    ]);
    assert_eq!(error_code(&r[5]), "ReceiptHandleExpired");
    assert_eq!(r[6], ok());
}

#[test]
fn change_message_visibility_replaces_the_end_of_the_timeout() {
    let change = |seconds: u32| json!({"Command": "ChangeMessageVisibility", "QueueName": "a", "ReceiptHandle": "r-1", "VisibilityTimeout": seconds});
    let r = commands(&[
        create("a"),
        send("a", "x"),
        receive("a"),
        tick(20),
        change(100),
        tick(99),
        counts("a"),
        tick(1),
        counts("a"),
        change(0),
    ]);
    assert_eq!(r[4], ok());
    assert_eq!(r[6]["Counts"]["InFlight"], json!(1));
    assert_eq!(r[8]["Counts"]["Visible"], json!(1));
    assert_eq!(error_code(&r[9]), "ReceiptHandleExpired");
}

#[test]
fn retention_removes_messages_in_every_state_and_is_not_undone() {
    let r = commands(&[
        create_with("a", json!({"RetentionSeconds": 60})),
        send("a", "x"),
        tick(59),
        counts("a"),
        tick(1),
        counts("a"),
        json!({"Command": "SetQueueAttributes", "QueueName": "a", "Attributes": {"RetentionSeconds": 1000}}),
        counts("a"),
    ]);
    assert_eq!(r[3]["Counts"]["Visible"], json!(1));
    assert_eq!(r[5]["Counts"]["Visible"], json!(0));
    assert_eq!(r[7]["Counts"]["Visible"], json!(0));
}

#[test]
fn lowering_retention_removes_old_messages_at_once() {
    let r = commands(&[
        create("a"),
        send("a", "x"),
        tick(100),
        send("a", "y"),
        json!({"Command": "SetQueueAttributes", "QueueName": "a", "Attributes": {"RetentionSeconds": 60}}),
        counts("a"),
    ]);
    assert_eq!(
        r[5]["Counts"],
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn set_queue_attributes_applies_the_rules_in_order() {
    let set = |queue: &str, attributes: Value| json!({"Command": "SetQueueAttributes", "QueueName": queue, "Attributes": attributes});
    let r = commands(&[
        create("a"),
        create("b"),
        set("a", json!({"Fifo": false})),
        set("a", json!({"ContentBasedDeduplication": true})),
        set("a", json!({"MaxReceiveCount": 2})),
        set(
            "a",
            json!({"DeadLetterQueue": "nope", "MaxReceiveCount": 2}),
        ),
        set("a", json!({"DeadLetterQueue": "a", "MaxReceiveCount": 2})),
        set("a", json!({"DeadLetterQueue": "b", "MaxReceiveCount": 2})),
        set("missing", json!({})),
        json!({"Command": "SetQueueAttributes", "QueueName": "a"}),
        counts("a"),
    ]);
    let codes: Vec<_> = r[2..10]
        .iter()
        .map(|x| x["error"]["code"].as_str().unwrap_or("ok"))
        .collect();
    assert_eq!(
        codes,
        [
            "UnsupportedOperation",
            "InvalidParameterValue",
            "InvalidParameterValue",
            "QueueDoesNotExist",
            "InvalidParameterValue",
            "ok",
            "QueueDoesNotExist",
            "MissingParameter"
        ]
    );
    assert_eq!(r[10]["Attributes"]["DeadLetterQueue"], json!("b"));
}

#[test]
fn deleting_a_queue_clears_dead_letter_references_and_frees_the_name() {
    let r = commands(&[
        create("dead"),
        create_with(
            "a",
            json!({"DeadLetterQueue": "dead", "MaxReceiveCount": 1}),
        ),
        send("dead", "old"),
        json!({"Command": "DeleteQueue", "QueueName": "dead"}),
        counts("a"),
        create("dead"),
        counts("dead"),
        json!({"Command": "DeleteQueue", "QueueName": "nope"}),
    ]);
    assert_eq!(r[3], ok());
    assert_eq!(r[4]["Attributes"].get("DeadLetterQueue"), None);
    assert_eq!(r[4]["Attributes"].get("MaxReceiveCount"), None);
    assert_eq!(
        r[6]["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(error_code(&r[7]), "QueueDoesNotExist");
}

#[test]
fn purge_removes_everything_but_keeps_sequence_and_deduplication() {
    let fifo = || create_with("q.fifo", json!({"Fifo": true}));
    let r = commands(&[
        fifo(),
        send_fifo("q.fifo", "a", "g", "d1"),
        json!({"Command": "PurgeQueue", "QueueName": "q.fifo"}),
        counts("q.fifo"),
        send_fifo("q.fifo", "b", "g", "d1"),
        send_fifo("q.fifo", "c", "g", "d2"),
    ]);
    assert_eq!(r[2], ok());
    assert_eq!(
        r[3]["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(
        r[4],
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
    assert_eq!(
        r[5],
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2})
    );
}

#[test]
fn send_message_checks_queue_type_rules() {
    let r = commands(&[
        create("std"),
        create_with("q.fifo", json!({"Fifo": true})),
        json!({"Command": "SendMessage", "QueueName": "std", "Body": "x", "GroupId": "g"}),
        json!({"Command": "SendMessage", "QueueName": "q.fifo", "Body": "x", "DelaySeconds": 1}),
        json!({"Command": "SendMessage", "QueueName": "q.fifo", "Body": "x"}),
        json!({"Command": "SendMessage", "QueueName": "q.fifo", "Body": "x", "GroupId": "g"}),
        json!({"Command": "SendMessage", "QueueName": "nope", "Body": ""}),
        json!({"Command": "SendMessage", "QueueName": "nope", "Body": "x"}),
    ]);
    let codes: Vec<_> = r[2..].iter().map(error_code).collect();
    assert_eq!(
        codes,
        [
            "UnsupportedOperation",
            "UnsupportedOperation",
            "MissingParameter",
            "MissingParameter",
            "InvalidParameterValue",
            "QueueDoesNotExist"
        ]
    );
}

#[test]
fn content_based_deduplication_uses_the_body() {
    let send_body = |body: &str| json!({"Command": "SendMessage", "QueueName": "q.fifo", "Body": body, "GroupId": "g"});
    let r = commands(&[
        create_with(
            "q.fifo",
            json!({"Fifo": true, "ContentBasedDeduplication": true}),
        ),
        send_body("same"),
        send_body("same"),
        send_body("other"),
    ]);
    assert_eq!(
        r[1],
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
    assert_eq!(r[2], r[1]);
    assert_eq!(
        r[3],
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2})
    );
}

#[test]
fn deduplication_window_is_300_seconds_and_duplicates_do_not_extend_it() {
    let dup = || send_fifo("q.fifo", "x", "g", "d");
    let r = commands(&[
        create_with("q.fifo", json!({"Fifo": true})),
        dup(),
        tick(299),
        dup(),
        tick(1),
        dup(),
        tick(299),
        dup(),
    ]);
    assert_eq!(r[1]["MessageId"], json!("m-1"));
    assert_eq!(r[3]["MessageId"], json!("m-1"));
    assert_eq!(
        r[5],
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2})
    );
    assert_eq!(r[7]["MessageId"], json!("m-2"));
}

#[test]
fn deduplication_outlives_the_message() {
    let r = commands(&[
        create_with("q.fifo", json!({"Fifo": true})),
        send_fifo("q.fifo", "x", "g", "d"),
        receive("q.fifo"),
        json!({"Command": "DeleteMessage", "QueueName": "q.fifo", "ReceiptHandle": "r-1"}),
        send_fifo("q.fifo", "y", "g", "d"),
        counts("q.fifo"),
    ]);
    assert_eq!(r[4]["MessageId"], json!("m-1"));
    assert_eq!(
        r[5]["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn fifo_delivers_one_message_per_group_and_blocks_busy_groups() {
    let r = commands(&[
        create_with("q.fifo", json!({"Fifo": true})),
        send_fifo("q.fifo", "a1", "A", "1"),
        send_fifo("q.fifo", "a2", "A", "2"),
        send_fifo("q.fifo", "b1", "B", "3"),
        receive_n("q.fifo", 10),
        receive_n("q.fifo", 10),
    ]);
    assert_eq!(bodies(&r[4]), ["a1", "b1"]);
    assert_eq!(r[4]["Messages"][0]["GroupId"], json!("A"));
    assert_eq!(r[4]["Messages"][1]["SequenceNumber"], json!(3));
    assert_eq!(bodies(&r[5]), Vec::<&str>::new());
}

#[test]
fn fifo_group_is_blocked_by_an_earlier_delayed_or_in_flight_message_until_it_is_deleted() {
    let r = commands(&[
        create_with("q.fifo", json!({"Fifo": true, "VisibilityTimeout": 5})),
        send_fifo("q.fifo", "a1", "A", "1"),
        send_fifo("q.fifo", "a2", "A", "2"),
        receive("q.fifo"),
        tick(5),
        receive("q.fifo"),
        json!({"Command": "DeleteMessage", "QueueName": "q.fifo", "ReceiptHandle": "r-2"}),
        receive("q.fifo"),
    ]);
    assert_eq!(bodies(&r[3]), ["a1"]);
    assert_eq!(bodies(&r[5]), ["a1"]);
    assert_eq!(r[5]["Messages"][0]["ReceiveCount"], json!(2));
    assert_eq!(bodies(&r[7]), ["a2"]);
}

#[test]
fn messages_move_to_the_dead_letter_queue_after_max_receive_count() {
    let r = commands(&[
        create("dead"),
        create_with(
            "a",
            json!({"DeadLetterQueue": "dead", "MaxReceiveCount": 2, "VisibilityTimeout": 1}),
        ),
        send("a", "x"),
        receive("a"),
        tick(1),
        receive("a"),
        tick(1),
        receive("a"),
        counts("a"),
        receive("dead"),
    ]);
    assert_eq!(r[5]["Messages"][0]["ReceiveCount"], json!(2));
    assert_eq!(r[7], json!({"ok": true, "Messages": []}));
    assert_eq!(
        r[8]["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(
        r[9]["Messages"][0],
        json!({
            "MessageId": "m-1", "ReceiptHandle": "r-3", "Body": "x",
            "ReceiveCount": 1, "SentAt": 0, "FirstReceivedAt": 2
        })
    );
}

#[test]
fn a_dead_lettered_message_is_not_checked_against_deduplication_ids() {
    let fifo_to_dead = json!({"Fifo": true, "DeadLetterQueue": "dead.fifo", "MaxReceiveCount": 1, "VisibilityTimeout": 0});
    let r = commands(&[
        create_with("dead.fifo", json!({"Fifo": true})),
        send_fifo("dead.fifo", "first", "G", "x"),
        create_with("a.fifo", fifo_to_dead),
        send_fifo("a.fifo", "moved", "G", "x"),
        receive("a.fifo"),
        receive("a.fifo"),
        receive_n("dead.fifo", 10),
    ]);
    assert_eq!(bodies(&r[6]), ["first"]);
    let r = commands(&[
        create_with("dead.fifo", json!({"Fifo": true, "VisibilityTimeout": 0})),
        create_with(
            "a.fifo",
            json!({"Fifo": true, "DeadLetterQueue": "dead.fifo", "MaxReceiveCount": 1, "VisibilityTimeout": 0}),
        ),
        send_fifo("a.fifo", "moved", "G", "x"),
        receive("a.fifo"),
        receive("a.fifo"),
        receive("dead.fifo"),
    ]);
    assert_eq!(bodies(&r[5]), ["moved"]);
}

#[test]
fn a_moved_message_that_is_too_old_for_the_dead_letter_queue_is_removed_on_arrival() {
    let r = commands(&[
        create_with("dead", json!({"RetentionSeconds": 60})),
        create_with(
            "a",
            json!({"DeadLetterQueue": "dead", "MaxReceiveCount": 1, "VisibilityTimeout": 0}),
        ),
        send("a", "x"),
        receive("a"),
        tick(61),
        receive("a"),
        counts("dead"),
    ]);
    assert_eq!(
        r[6]["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn a_moved_message_continues_the_sequence_of_the_dead_letter_queue() {
    let r = commands(&[
        create_with("dead.fifo", json!({"Fifo": true})),
        send_fifo("dead.fifo", "first", "G", "x"),
        create_with(
            "a.fifo",
            json!({"Fifo": true, "DeadLetterQueue": "dead.fifo", "MaxReceiveCount": 1, "VisibilityTimeout": 0}),
        ),
        send_fifo("a.fifo", "moved", "H", "x"),
        receive("a.fifo"),
        receive("a.fifo"),
        receive_n("dead.fifo", 10),
    ]);
    assert_eq!(bodies(&r[6]), ["first", "moved"]);
    assert_eq!(r[6]["Messages"][1]["SequenceNumber"], json!(2));
    assert_eq!(r[6]["Messages"][1]["GroupId"], json!("H"));
    assert_eq!(r[6]["Messages"][1]["ReceiveCount"], json!(1));
}

#[test]
fn dead_lettering_does_not_count_toward_max_messages() {
    let r = commands(&[
        create("dead"),
        create_with(
            "a",
            json!({"DeadLetterQueue": "dead", "MaxReceiveCount": 1, "VisibilityTimeout": 0}),
        ),
        send("a", "old"),
        receive("a"),
        send("a", "new"),
        receive("a"),
    ]);
    assert_eq!(bodies(&r[5]), ["new"]);
}

#[test]
fn a_full_queue_rejects_sends_until_purged() {
    let mut cmds = vec![create("a")];
    cmds.extend((0..100_000).map(|_| send("a", "x")));
    cmds.push(send("a", "x"));
    cmds.push(json!({"Command": "PurgeQueue", "QueueName": "a"}));
    cmds.push(send("a", "x"));
    let r = commands(&cmds);
    assert!(r[1..100_001].iter().all(|x| x["ok"] == json!(true)));
    assert_eq!(error_code(&r[100_001]), "LimitExceeded");
    assert_eq!(r[100_003]["MessageId"], json!("m-100001"));
}

#[test]
fn the_binary_answers_stdin_and_exits_zero() {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let mut child = Command::new(env!("CARGO_BIN_EXE_miniq"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("the miniq binary starts");
    child
        .stdin
        .take()
        .expect("stdin is piped")
        .write_all(b"{\"Command\": \"Tick\", \"Seconds\": 7}")
        .expect("stdin accepts the line");
    let output = child
        .wait_with_output()
        .expect("the binary exits at end of input");
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "{\"ok\":true,\"Now\":7}\n"
    );
}
