//! End-to-end behavior of the line protocol, driven through `miniq::run`.

use std::io::Cursor;

use serde_json::{Value, json};

/// Feeds `input` to miniq and returns the raw response lines.
fn run_bytes(input: &[u8]) -> Vec<String> {
    let mut output = Vec::new();
    miniq::run(Cursor::new(input), &mut output).expect("in-memory io does not fail");
    let text = String::from_utf8(output).expect("responses are UTF-8");
    assert!(
        text.is_empty() || text.ends_with('\n'),
        "every response ends in a newline"
    );
    text.lines().map(str::to_owned).collect()
}

/// Runs one command per element and returns the parsed responses.
fn session(commands: &[Value]) -> Vec<Value> {
    let input: String = commands.iter().map(|c| format!("{c}\n")).collect();
    run_bytes(input.as_bytes())
        .iter()
        .map(|line| serde_json::from_str(line).expect("each response is one JSON object"))
        .collect()
}

fn error(code: &str) -> Value {
    json!({"ok": false, "error": {"code": code}})
}

/// The response without the free-text error message.
fn without_message(mut response: Value) -> Value {
    if let Some(error) = response.get_mut("error").and_then(Value::as_object_mut) {
        error.remove("message");
    }
    response
}

fn assert_session(commands: &[Value], expected: &[Value]) {
    let actual: Vec<Value> = session(commands).into_iter().map(without_message).collect();
    assert_eq!(actual, expected);
}

fn create(name: &str, attributes: Value) -> Value {
    json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attributes})
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

fn tick(seconds: u32) -> Value {
    json!({"Command": "Tick", "Seconds": seconds})
}

fn ok() -> Value {
    json!({"ok": true})
}

#[test]
fn every_line_gets_exactly_one_response_in_order() {
    let lines = run_bytes(
        b"\n{\"Command\":\"Tick\",\"Seconds\":2}\nnot json\n{\"Command\":\"Tick\",\"Seconds\":3}",
    );
    assert_eq!(
        lines[1..2].concat() + &lines[3..].concat(),
        r#"{"ok":true,"Now":2}{"ok":true,"Now":5}"#
    );
    assert_eq!(lines.len(), 4);
    let codes: Vec<Value> = [&lines[0], &lines[2]]
        .iter()
        .map(|line| {
            serde_json::from_str::<Value>(line).expect("response is JSON")["error"]["code"].clone()
        })
        .collect();
    assert_eq!(codes, [json!("InvalidRequest"), json!("InvalidRequest")]);
}

#[test]
fn empty_input_and_a_lone_newline_are_handled() {
    assert_eq!(run_bytes(b""), Vec::<String>::new());
    assert_eq!(run_bytes(b"\n").len(), 1);
}

#[test]
fn invalid_utf8_is_an_invalid_request_and_the_next_line_still_runs() {
    let lines = run_bytes(b"{\"Command\":\"Tick\",\"Seconds\":1,\"X\":\"\xc3\x28\"}\n{\"Command\":\"Tick\",\"Seconds\":1}\n");
    let parsed: Vec<Value> = lines
        .iter()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(without_message(parsed[0].clone()), error("InvalidRequest"));
    assert_eq!(parsed[1], json!({"ok": true, "Now": 1}));
}

#[test]
fn error_precedence_follows_the_specified_order() {
    assert_session(
        &[
            json!({"Command": "SendMessage", "QueueName": 7}),
            json!({"Command": "SendMessage", "QueueName": "missing", "Body": ""}),
            json!({"Command": "SendMessage", "QueueName": "missing", "Body": "x"}),
            json!({"Command": "PurgeQueue", "QueueName": "has space"}),
            create("bad name", json!(5)),
            create("a.fifo", json!({})),
            create("a", json!({"Fifo": true})),
            create("a", json!({"ContentBasedDeduplication": true})),
            create("a", json!({"DeadLetterQueue": "x"})),
            create("a", json!({"MaxReceiveCount": 1})),
            create("a", json!({"DeadLetterQueue": "x", "MaxReceiveCount": 1})),
        ],
        &[
            error("MissingParameter"),
            error("InvalidParameterValue"),
            error("QueueDoesNotExist"),
            error("QueueDoesNotExist"),
            error("QueueNameInvalid"),
            error("QueueNameInvalid"),
            error("QueueNameInvalid"),
            error("InvalidParameterValue"),
            error("InvalidParameterValue"),
            error("InvalidParameterValue"),
            error("QueueDoesNotExist"),
        ],
    );
}

#[test]
fn create_queue_is_idempotent_only_for_equal_attributes() {
    assert_session(
        &[
            create("jobs", json!({})),
            create(
                "jobs",
                json!({"VisibilityTimeout": 30, "DelaySeconds": null}),
            ),
            create("jobs", json!({"VisibilityTimeout": 31})),
            create("jobs", json!({"RetentionSeconds": 345600})),
            create("jobs.fifo", json!({"Fifo": true})),
            create(
                "jobs.fifo",
                json!({"Fifo": true, "ContentBasedDeduplication": true}),
            ),
            create(
                "jobs",
                json!({"Fifo": false, "ContentBasedDeduplication": false}),
            ),
        ],
        &[
            ok(),
            ok(),
            error("QueueAlreadyExists"),
            ok(),
            ok(),
            error("QueueAlreadyExists"),
            ok(),
        ],
    );
}

#[test]
fn dead_letter_configuration_rules_on_create() {
    assert_session(
        &[
            create("dead", json!({})),
            create("dead.fifo", json!({"Fifo": true})),
            create(
                "q.fifo",
                json!({"Fifo": true, "DeadLetterQueue": "dead", "MaxReceiveCount": 2}),
            ),
            create(
                "q.fifo",
                json!({"Fifo": true, "DeadLetterQueue": "dead.fifo", "MaxReceiveCount": 2}),
            ),
            create("q", json!({"DeadLetterQueue": "q", "MaxReceiveCount": 2})),
            create(
                "q",
                json!({"DeadLetterQueue": "dead", "MaxReceiveCount": 2}),
            ),
            json!({"Command": "GetQueueAttributes", "QueueName": "q"}),
            create(
                "q",
                json!({"DeadLetterQueue": "dead", "MaxReceiveCount": 3}),
            ),
        ],
        &[
            ok(),
            ok(),
            error("InvalidParameterValue"),
            ok(),
            error("QueueDoesNotExist"),
            ok(),
            json!({"ok": true,
                   "Attributes": {"VisibilityTimeout": 30, "DelaySeconds": 0,
                                  "RetentionSeconds": 345600, "Fifo": false,
                                  "ContentBasedDeduplication": false,
                                  "DeadLetterQueue": "dead", "MaxReceiveCount": 2},
                   "Counts": {"Visible": 0, "InFlight": 0, "Delayed": 0}}),
            error("QueueAlreadyExists"),
        ],
    );
}

#[test]
fn set_queue_attributes_rules_and_effects() {
    let set = |name: &str, attributes: Value| json!({"Command": "SetQueueAttributes", "QueueName": name, "Attributes": attributes});
    assert_session(
        &[
            create("q", json!({})),
            create("dead", json!({})),
            create("f.fifo", json!({"Fifo": true})),
            set("q", json!({"Fifo": false})),
            set("q", json!({"ContentBasedDeduplication": true})),
            set("q", json!({"DeadLetterQueue": "dead"})),
            set(
                "q",
                json!({"DeadLetterQueue": "nope", "MaxReceiveCount": 1}),
            ),
            set("q", json!({"DeadLetterQueue": "q", "MaxReceiveCount": 1})),
            set(
                "q",
                json!({"DeadLetterQueue": "f.fifo", "MaxReceiveCount": 1}),
            ),
            set(
                "q",
                json!({"DeadLetterQueue": "dead", "MaxReceiveCount": 5, "DelaySeconds": 7}),
            ),
            set("q", json!({"VisibilityTimeout": 0})),
            set("f.fifo", json!({"ContentBasedDeduplication": true})),
            set("missing", json!({"Fifo": true})),
            json!({"Command": "SetQueueAttributes", "QueueName": "q"}),
            json!({"Command": "GetQueueAttributes", "QueueName": "q"}),
        ],
        &[
            ok(),
            ok(),
            ok(),
            error("UnsupportedOperation"),
            error("InvalidParameterValue"),
            error("InvalidParameterValue"),
            error("QueueDoesNotExist"),
            error("InvalidParameterValue"),
            error("InvalidParameterValue"),
            ok(),
            ok(),
            ok(),
            error("QueueDoesNotExist"),
            error("MissingParameter"),
            json!({"ok": true,
                   "Attributes": {"VisibilityTimeout": 0, "DelaySeconds": 7,
                                  "RetentionSeconds": 345600, "Fifo": false,
                                  "ContentBasedDeduplication": false,
                                  "DeadLetterQueue": "dead", "MaxReceiveCount": 5},
                   "Counts": {"Visible": 0, "InFlight": 0, "Delayed": 0}}),
        ],
    );
}

#[test]
fn list_queues_sorts_by_byte_value_and_filters_by_prefix() {
    let list = |prefix: Value| json!({"Command": "ListQueues", "Prefix": prefix});
    assert_session(
        &[
            create("b", json!({})),
            create("a_x", json!({})),
            create("a-x", json!({})),
            create("a0", json!({})),
            create("aB", json!({})),
            create("a.fifo", json!({"Fifo": true})),
            json!({"Command": "ListQueues"}),
            list(json!("a")),
            list(json!("a.")),
            list(json!("")),
            list(json!("zzz")),
            list(json!(null)),
            list(json!(1)),
        ],
        &[
            ok(),
            ok(),
            ok(),
            ok(),
            ok(),
            ok(),
            json!({"ok": true, "QueueNames": ["a-x", "a.fifo", "a0", "aB", "a_x", "b"]}),
            json!({"ok": true, "QueueNames": ["a-x", "a.fifo", "a0", "aB", "a_x"]}),
            json!({"ok": true, "QueueNames": ["a.fifo"]}),
            json!({"ok": true, "QueueNames": ["a-x", "a.fifo", "a0", "aB", "a_x", "b"]}),
            json!({"ok": true, "QueueNames": []}),
            json!({"ok": true, "QueueNames": ["a-x", "a.fifo", "a0", "aB", "a_x", "b"]}),
            error("InvalidParameterValue"),
        ],
    );
}

#[test]
fn standard_queue_send_receive_and_visibility() {
    assert_session(
        &[
            create("q", json!({"VisibilityTimeout": 10})),
            send("q", "one"),
            json!({"Command": "SendMessage", "QueueName": "q", "Body": "two", "DelaySeconds": 5}),
            receive("q"),
            json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 5}),
            tick(5),
            json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 5, "VisibilityTimeout": 0}),
            json!({"Command": "GetQueueAttributes", "QueueName": "q"}),
            tick(5),
            json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 10}),
        ],
        &[
            ok(),
            json!({"ok": true, "MessageId": "m-1"}),
            json!({"ok": true, "MessageId": "m-2"}),
            json!({"ok": true, "Messages": [{"MessageId": "m-1", "ReceiptHandle": "r-1", "Body": "one",
                    "ReceiveCount": 1, "SentAt": 0, "FirstReceivedAt": 0}]}),
            json!({"ok": true, "Messages": []}),
            json!({"ok": true, "Now": 5}),
            json!({"ok": true, "Messages": [{"MessageId": "m-2", "ReceiptHandle": "r-2", "Body": "two",
                    "ReceiveCount": 1, "SentAt": 0, "FirstReceivedAt": 5}]}),
            json!({"ok": true,
                   "Attributes": {"VisibilityTimeout": 10, "DelaySeconds": 0,
                                  "RetentionSeconds": 345600, "Fifo": false,
                                  "ContentBasedDeduplication": false},
                   "Counts": {"Visible": 1, "InFlight": 1, "Delayed": 0}}),
            json!({"ok": true, "Now": 10}),
            json!({"ok": true, "Messages": [
                {"MessageId": "m-1", "ReceiptHandle": "r-3", "Body": "one",
                 "ReceiveCount": 2, "SentAt": 0, "FirstReceivedAt": 0},
                {"MessageId": "m-2", "ReceiptHandle": "r-4", "Body": "two",
                 "ReceiveCount": 2, "SentAt": 0, "FirstReceivedAt": 5}]}),
        ],
    );
}

#[test]
fn send_rules_for_standard_and_fifo_queues() {
    let with = |queue: &str, extra: Value| {
        let mut command = send(queue, "x");
        command
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        command
    };
    assert_session(
        &[
            create("s", json!({})),
            create("f.fifo", json!({"Fifo": true})),
            create(
                "c.fifo",
                json!({"Fifo": true, "ContentBasedDeduplication": true}),
            ),
            with("s", json!({"GroupId": "g"})),
            with("s", json!({"DeduplicationId": "d"})),
            with("f.fifo", json!({"DelaySeconds": 0, "GroupId": "g"})),
            with("f.fifo", json!({})),
            with("f.fifo", json!({"GroupId": "g"})),
            with("c.fifo", json!({"GroupId": "g"})),
            with("c.fifo", json!({"GroupId": "h"})),
            with(
                "c.fifo",
                json!({"GroupId": "g", "DeduplicationId": "other"}),
            ),
        ],
        &[
            ok(),
            ok(),
            ok(),
            error("UnsupportedOperation"),
            error("UnsupportedOperation"),
            error("UnsupportedOperation"),
            error("MissingParameter"),
            error("MissingParameter"),
            json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1}),
            json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1}),
            json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2}),
        ],
    );
}

#[test]
fn deduplication_window_ignores_duplicates_and_outlives_the_message() {
    assert_session(
        &[
            create("q.fifo", json!({"Fifo": true})),
            send_fifo("q.fifo", "a", "g", "d"),
            tick(299),
            send_fifo("q.fifo", "b", "other-group", "d"),
            json!({"Command": "PurgeQueue", "QueueName": "q.fifo"}),
            send_fifo("q.fifo", "c", "g", "d"),
            tick(1),
            send_fifo("q.fifo", "d", "g", "d"),
            tick(299),
            send_fifo("q.fifo", "e", "g", "d"),
            tick(1),
            send_fifo("q.fifo", "f", "g", "d"),
        ],
        &[
            ok(),
            json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1}),
            json!({"ok": true, "Now": 299}),
            json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1}),
            ok(),
            json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1}),
            json!({"ok": true, "Now": 300}),
            json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2}),
            json!({"ok": true, "Now": 599}),
            json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2}),
            json!({"ok": true, "Now": 600}),
            json!({"ok": true, "MessageId": "m-3", "SequenceNumber": 3}),
        ],
    );
}

#[test]
fn fifo_receive_gives_one_message_per_group_and_keeps_order_within_a_group() {
    assert_session(
        &[
            create("q.fifo", json!({"Fifo": true})),
            send_fifo("q.fifo", "a1", "a", "1"),
            send_fifo("q.fifo", "a2", "a", "2"),
            send_fifo("q.fifo", "b1", "b", "3"),
            json!({"Command": "ReceiveMessage", "QueueName": "q.fifo", "MaxMessages": 10}),
            json!({"Command": "DeleteMessage", "QueueName": "q.fifo", "ReceiptHandle": "r-1"}),
            json!({"Command": "ReceiveMessage", "QueueName": "q.fifo", "MaxMessages": 10}),
        ],
        &[
            ok(),
            json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1}),
            json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2}),
            json!({"ok": true, "MessageId": "m-3", "SequenceNumber": 3}),
            json!({"ok": true, "Messages": [
                {"MessageId": "m-1", "ReceiptHandle": "r-1", "Body": "a1", "ReceiveCount": 1,
                 "SentAt": 0, "FirstReceivedAt": 0, "GroupId": "a", "SequenceNumber": 1},
                {"MessageId": "m-3", "ReceiptHandle": "r-2", "Body": "b1", "ReceiveCount": 1,
                 "SentAt": 0, "FirstReceivedAt": 0, "GroupId": "b", "SequenceNumber": 3}]}),
            ok(),
            json!({"ok": true, "Messages": [
                {"MessageId": "m-2", "ReceiptHandle": "r-3", "Body": "a2", "ReceiveCount": 1,
                 "SentAt": 0, "FirstReceivedAt": 0, "GroupId": "a", "SequenceNumber": 2}]}),
        ],
    );
}

#[test]
fn receipt_handle_errors_distinguish_never_issued_from_no_longer_usable() {
    let delete = |queue: &str, handle: &str| json!({"Command": "DeleteMessage", "QueueName": queue, "ReceiptHandle": handle});
    let change = |queue: &str, handle: &str, timeout: u32| {
        json!({"Command": "ChangeMessageVisibility", "QueueName": queue,
               "ReceiptHandle": handle, "VisibilityTimeout": timeout})
    };
    assert_session(
        &[
            create("q", json!({"VisibilityTimeout": 10})),
            create("other", json!({})),
            send("q", "a"),
            receive("q"),
            delete("q", "r-01"),
            delete("q", "R-1"),
            delete("q", "r-1 "),
            delete("q", "r-2"),
            delete("other", "r-1"),
            delete("missing", "bogus"),
            change("q", "r-1", 20),
            tick(15),
            delete("q", "r-1"),
            tick(5),
            delete("q", "r-1"),
            receive("q"),
            delete("q", "r-1"),
            change("q", "r-2", 0),
            delete("q", "r-2"),
            receive("q"),
            delete("q", "r-3"),
            delete("q", "r-3"),
        ],
        &[
            ok(),
            ok(),
            json!({"ok": true, "MessageId": "m-1"}),
            json!({"ok": true, "Messages": [{"MessageId": "m-1", "ReceiptHandle": "r-1", "Body": "a",
                    "ReceiveCount": 1, "SentAt": 0, "FirstReceivedAt": 0}]}),
            error("ReceiptHandleInvalid"),
            error("ReceiptHandleInvalid"),
            error("ReceiptHandleInvalid"),
            error("ReceiptHandleInvalid"),
            error("ReceiptHandleExpired"),
            error("QueueDoesNotExist"),
            ok(),
            json!({"ok": true, "Now": 15}),
            ok(),
            json!({"ok": true, "Now": 20}),
            error("ReceiptHandleExpired"),
            json!({"ok": true, "Messages": []}),
            error("ReceiptHandleExpired"),
            error("ReceiptHandleInvalid"),
            error("ReceiptHandleInvalid"),
            json!({"ok": true, "Messages": []}),
            error("ReceiptHandleInvalid"),
            error("ReceiptHandleInvalid"),
        ],
    );
}

#[test]
fn zero_visibility_timeout_makes_the_handle_unusable_at_once() {
    assert_session(
        &[
            create("q", json!({})),
            send("q", "a"),
            json!({"Command": "ReceiveMessage", "QueueName": "q", "VisibilityTimeout": 0}),
            json!({"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": "r-1"}),
            json!({"Command": "ChangeMessageVisibility", "QueueName": "q",
                   "ReceiptHandle": "r-1", "VisibilityTimeout": 5}),
        ],
        &[
            ok(),
            json!({"ok": true, "MessageId": "m-1"}),
            json!({"ok": true, "Messages": [{"MessageId": "m-1", "ReceiptHandle": "r-1", "Body": "a",
                    "ReceiveCount": 1, "SentAt": 0, "FirstReceivedAt": 0}]}),
            error("ReceiptHandleExpired"),
            error("ReceiptHandleExpired"),
        ],
    );
}

#[test]
fn messages_over_the_receive_limit_move_to_the_dead_letter_queue() {
    assert_session(
        &[
            create("dead", json!({"RetentionSeconds": 60})),
            create(
                "q",
                json!({"VisibilityTimeout": 0, "DeadLetterQueue": "dead", "MaxReceiveCount": 2}),
            ),
            send("q", "a"),
            receive("q"),
            receive("q"),
            tick(1),
            receive("q"),
            json!({"Command": "GetQueueAttributes", "QueueName": "dead"}),
            receive("dead"),
        ],
        &[
            ok(),
            ok(),
            json!({"ok": true, "MessageId": "m-1"}),
            json!({"ok": true, "Messages": [{"MessageId": "m-1", "ReceiptHandle": "r-1", "Body": "a",
                    "ReceiveCount": 1, "SentAt": 0, "FirstReceivedAt": 0}]}),
            json!({"ok": true, "Messages": [{"MessageId": "m-1", "ReceiptHandle": "r-2", "Body": "a",
                    "ReceiveCount": 2, "SentAt": 0, "FirstReceivedAt": 0}]}),
            json!({"ok": true, "Now": 1}),
            json!({"ok": true, "Messages": []}),
            json!({"ok": true,
                   "Attributes": {"VisibilityTimeout": 30, "DelaySeconds": 0,
                                  "RetentionSeconds": 60, "Fifo": false,
                                  "ContentBasedDeduplication": false},
                   "Counts": {"Visible": 1, "InFlight": 0, "Delayed": 0}}),
            // The dead-letter move reset the counters; this is a fresh first delivery.
            json!({"ok": true, "Messages": [{"MessageId": "m-1", "ReceiptHandle": "r-3", "Body": "a",
                    "ReceiveCount": 1, "SentAt": 0, "FirstReceivedAt": 1}]}),
        ],
    );
}

#[test]
fn a_dead_lettered_message_older_than_the_target_retention_vanishes_on_arrival() {
    assert_session(
        &[
            create("dead", json!({"RetentionSeconds": 60})),
            create(
                "q",
                json!({"VisibilityTimeout": 0, "DeadLetterQueue": "dead", "MaxReceiveCount": 1}),
            ),
            send("q", "a"),
            receive("q"),
            tick(61),
            receive("q"),
            json!({"Command": "GetQueueAttributes", "QueueName": "dead"}),
        ],
        &[
            ok(),
            ok(),
            json!({"ok": true, "MessageId": "m-1"}),
            json!({"ok": true, "Messages": [{"MessageId": "m-1", "ReceiptHandle": "r-1", "Body": "a",
                    "ReceiveCount": 1, "SentAt": 0, "FirstReceivedAt": 0}]}),
            json!({"ok": true, "Now": 61}),
            json!({"ok": true, "Messages": []}),
            json!({"ok": true,
                   "Attributes": {"VisibilityTimeout": 30, "DelaySeconds": 0,
                                  "RetentionSeconds": 60, "Fifo": false,
                                  "ContentBasedDeduplication": false},
                   "Counts": {"Visible": 0, "InFlight": 0, "Delayed": 0}}),
        ],
    );
}

#[test]
fn deleting_a_dead_letter_queue_clears_the_configuration_of_its_users() {
    assert_session(
        &[
            create("dead", json!({})),
            create(
                "q",
                json!({"DeadLetterQueue": "dead", "MaxReceiveCount": 2}),
            ),
            json!({"Command": "DeleteQueue", "QueueName": "dead"}),
            json!({"Command": "GetQueueAttributes", "QueueName": "q"}),
            create("dead", json!({})),
            json!({"Command": "DeleteQueue", "QueueName": "dead"}),
            json!({"Command": "DeleteQueue", "QueueName": "dead"}),
        ],
        &[
            ok(),
            ok(),
            ok(),
            json!({"ok": true,
                   "Attributes": {"VisibilityTimeout": 30, "DelaySeconds": 0,
                                  "RetentionSeconds": 345600, "Fifo": false,
                                  "ContentBasedDeduplication": false},
                   "Counts": {"Visible": 0, "InFlight": 0, "Delayed": 0}}),
            ok(),
            ok(),
            error("QueueDoesNotExist"),
        ],
    );
}

#[test]
fn retention_removes_messages_at_once_when_lowered_and_does_not_bring_them_back() {
    let counts = |queue: &str| json!({"Command": "GetQueueAttributes", "QueueName": queue});
    let responses = session(&[
        create("q", json!({})),
        send("q", "a"),
        tick(100),
        send("q", "b"),
        json!({"Command": "SetQueueAttributes", "QueueName": "q",
               "Attributes": {"RetentionSeconds": 100}}),
        counts("q"),
        json!({"Command": "SetQueueAttributes", "QueueName": "q",
               "Attributes": {"RetentionSeconds": 1000}}),
        counts("q"),
        tick(999),
        counts("q"),
        tick(1),
        counts("q"),
    ]);
    let visible: Vec<&Value> = [5, 7, 9, 11]
        .iter()
        .map(|&i| &responses[i]["Counts"]["Visible"])
        .collect();
    assert_eq!(visible, [1, 1, 1, 0]);
}

#[test]
fn a_new_queue_under_a_deleted_name_shares_nothing_with_the_old_one() {
    assert_session(
        &[
            create("q", json!({"VisibilityTimeout": 100})),
            send("q", "a"),
            receive("q"),
            json!({"Command": "DeleteQueue", "QueueName": "q"}),
            create("q", json!({"VisibilityTimeout": 100})),
            json!({"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": "r-1"}),
            receive("q"),
        ],
        &[
            ok(),
            json!({"ok": true, "MessageId": "m-1"}),
            json!({"ok": true, "Messages": [{"MessageId": "m-1", "ReceiptHandle": "r-1", "Body": "a",
                    "ReceiveCount": 1, "SentAt": 0, "FirstReceivedAt": 0}]}),
            ok(),
            ok(),
            error("ReceiptHandleExpired"),
            json!({"ok": true, "Messages": []}),
        ],
    );
}

#[test]
fn queue_and_message_limits_are_enforced() {
    let mut commands: Vec<Value> = (0..1000)
        .map(|n| create(&format!("q{n}"), json!({})))
        .collect();
    commands.push(create("one-too-many", json!({})));
    commands.push(create("q0", json!({})));
    let responses = session(&commands);
    assert_eq!(responses[..1000], vec![ok(); 1000][..]);
    assert_eq!(
        without_message(responses[1000].clone()),
        error("LimitExceeded")
    );
    assert_eq!(responses[1001], ok());

    let mut commands = vec![create("q", json!({}))];
    commands.extend((0..100_000).map(|_| send("q", "x")));
    commands.push(send("q", "x"));
    commands.push(json!({"Command": "PurgeQueue", "QueueName": "q"}));
    commands.push(send("q", "x"));
    let responses = session(&commands);
    assert_eq!(
        responses[100_000],
        json!({"ok": true, "MessageId": "m-100000"})
    );
    assert_eq!(
        without_message(responses[100_001].clone()),
        error("LimitExceeded")
    );
    assert_eq!(
        responses[100_003],
        json!({"ok": true, "MessageId": "m-100001"})
    );
}

#[test]
fn unknown_parameters_and_unrelated_nulls_are_ignored() {
    assert_session(
        &[
            json!({"Command": "Tick", "Seconds": 1, "Extra": {"deep": [1, 2]}}),
            json!({"Command": "ListQueues", "Prefix": null, "Whatever": null}),
        ],
        &[
            json!({"ok": true, "Now": 1}),
            json!({"ok": true, "QueueNames": []}),
        ],
    );
}
