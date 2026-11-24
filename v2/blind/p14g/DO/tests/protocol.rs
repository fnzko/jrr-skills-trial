//! End-to-end behavior of the protocol: every test feeds command lines to `miniq::run` and
//! compares the response lines.

use std::io::Cursor;

use serde_json::{Value, json};

fn run_bytes(input: &[u8]) -> Vec<Value> {
    let mut output = Vec::new();
    miniq::run(Cursor::new(input), &mut output).expect("in-memory I/O does not fail");
    String::from_utf8(output)
        .expect("responses are UTF-8")
        .lines()
        .map(|line| serde_json::from_str(line).expect("every response line is JSON"))
        .collect()
}

fn session(commands: &[Value]) -> Vec<Value> {
    let input: String = commands.iter().map(|c| format!("{c}\n")).collect();
    let responses = run_bytes(input.as_bytes());
    assert_eq!(responses.len(), commands.len(), "one response per line");
    responses
}

fn ok() -> Value {
    json!({"ok": true})
}

fn code(response: &Value) -> &str {
    assert_eq!(response["ok"], json!(false), "{response}");
    response["error"]["code"]
        .as_str()
        .expect("error code is a string")
}

fn create(name: &str, attributes: Value) -> Value {
    json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attributes})
}

fn send(queue: &str, body: &str) -> Value {
    json!({"Command": "SendMessage", "QueueName": queue, "Body": body})
}

fn receive(queue: &str) -> Value {
    json!({"Command": "ReceiveMessage", "QueueName": queue, "MaxMessages": 10})
}

fn tick(seconds: u64) -> Value {
    json!({"Command": "Tick", "Seconds": seconds})
}

fn attributes(queue: &str) -> Value {
    json!({"Command": "GetQueueAttributes", "QueueName": queue})
}

fn message(
    id: u64,
    handle: u64,
    body: &str,
    receive_count: u64,
    sent_at: u64,
    first: u64,
) -> Value {
    json!({
        "MessageId": format!("m-{id}"), "ReceiptHandle": format!("r-{handle}"), "Body": body,
        "ReceiveCount": receive_count, "SentAt": sent_at, "FirstReceivedAt": first,
    })
}

fn fifo_message(base: Value, group: &str, sequence: u64) -> Value {
    let mut base = base;
    base["GroupId"] = json!(group);
    base["SequenceNumber"] = json!(sequence);
    base
}

fn messages(list: Vec<Value>) -> Value {
    json!({"ok": true, "Messages": list})
}

#[test]
fn every_line_gets_one_response_and_bad_lines_do_not_stop_the_session() {
    let input: &[u8] = b"\n{\"Command\":\"Tick\",\"Seconds\":1}\nnot json\n[]\n{\"Command\":5}\n{\"Command\":\"Nope\"}\n{\"Command\":\"Tick\",\"Seconds\":1}";
    let responses = run_bytes(input);
    let codes: Vec<&str> = responses
        .iter()
        .map(|r| {
            if r["ok"] == json!(true) {
                "ok"
            } else {
                code(r)
            }
        })
        .collect();
    assert_eq!(
        codes,
        [
            "InvalidRequest",
            "ok",
            "InvalidRequest",
            "InvalidRequest",
            "InvalidRequest",
            "InvalidRequest",
            "ok"
        ]
    );
    assert_eq!(responses[6], json!({"ok": true, "Now": 2}));
}

#[test]
fn invalid_utf8_is_an_invalid_request() {
    let responses = run_bytes(b"{\"Command\":\"Tick\",\"Seconds\":1,\"x\":\"\xff\"}\n");
    assert_eq!(code(&responses[0]), "InvalidRequest");
}

#[test]
fn empty_input_produces_no_output_and_a_blank_last_line_is_not_a_command() {
    assert_eq!(run_bytes(b""), Vec::<Value>::new());
    assert_eq!(run_bytes(b"\n").len(), 1);
}

#[test]
fn error_codes_follow_the_checking_order() {
    let cases: Vec<(&str, Value, &str)> = vec![
        (
            "missing before invalid",
            json!({"Command": "SendMessage", "QueueName": 5}),
            "MissingParameter",
        ),
        (
            "null is absent",
            json!({"Command": "SendMessage", "QueueName": "q", "Body": null}),
            "MissingParameter",
        ),
        (
            "wrong type",
            json!({"Command": "SendMessage", "QueueName": 5, "Body": "x"}),
            "InvalidParameterValue",
        ),
        (
            "value before lookup",
            json!({"Command": "SendMessage", "QueueName": "nope", "Body": ""}),
            "InvalidParameterValue",
        ),
        (
            "lookup",
            json!({"Command": "SendMessage", "QueueName": "nope", "Body": "x"}),
            "QueueDoesNotExist",
        ),
        (
            "lookup of the empty name",
            attributes(""),
            "QueueDoesNotExist",
        ),
        (
            "name checked first",
            create("bad name", json!(5)),
            "QueueNameInvalid",
        ),
        (
            "attributes not an object",
            create("ok", json!(5)),
            "InvalidParameterValue",
        ),
        (
            "suffix without Fifo",
            create("a.fifo", json!({})),
            "QueueNameInvalid",
        ),
        (
            "Fifo without suffix",
            create("a", json!({"Fifo": true})),
            "QueueNameInvalid",
        ),
        (
            "cbd on standard",
            create("a", json!({"ContentBasedDeduplication": true})),
            "InvalidParameterValue",
        ),
        (
            "unknown attribute",
            create("a", json!({"Colour": 1})),
            "InvalidParameterValue",
        ),
        (
            "attribute out of range",
            create("a", json!({"RetentionSeconds": 59})),
            "InvalidParameterValue",
        ),
        (
            "lone dead-letter queue",
            create("a", json!({"DeadLetterQueue": "q"})),
            "InvalidParameterValue",
        ),
        (
            "missing dead-letter queue",
            create("a", json!({"DeadLetterQueue": "zz", "MaxReceiveCount": 1})),
            "QueueDoesNotExist",
        ),
        (
            "fifo dead-letter mismatch",
            create(
                "a",
                json!({"DeadLetterQueue": "q", "MaxReceiveCount": 1, "Fifo": true}),
            ),
            "QueueNameInvalid",
        ),
        (
            "tick float",
            json!({"Command": "Tick", "Seconds": 30.0}),
            "InvalidParameterValue",
        ),
        (
            "tick exponent",
            json!({"Command": "Tick", "Seconds": 1e1}),
            "InvalidParameterValue",
        ),
        (
            "tick string",
            json!({"Command": "Tick", "Seconds": "30"}),
            "InvalidParameterValue",
        ),
        (
            "tick negative",
            json!({"Command": "Tick", "Seconds": -1}),
            "InvalidParameterValue",
        ),
        (
            "tick too large",
            json!({"Command": "Tick", "Seconds": 1209601}),
            "InvalidParameterValue",
        ),
        (
            "tick missing",
            json!({"Command": "Tick"}),
            "MissingParameter",
        ),
        (
            "set without attributes",
            json!({"Command": "SetQueueAttributes", "QueueName": "q"}),
            "MissingParameter",
        ),
        (
            "set Fifo",
            json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"Fifo": false}}),
            "UnsupportedOperation",
        ),
        (
            "set cbd on standard",
            json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"ContentBasedDeduplication": true}}),
            "InvalidParameterValue",
        ),
        (
            "set own dead-letter",
            json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"DeadLetterQueue": "q", "MaxReceiveCount": 1}}),
            "InvalidParameterValue",
        ),
        (
            "set missing dead-letter",
            json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"DeadLetterQueue": "zz", "MaxReceiveCount": 1}}),
            "QueueDoesNotExist",
        ),
        (
            "set mismatched dead-letter",
            json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"DeadLetterQueue": "f.fifo", "MaxReceiveCount": 1}}),
            "InvalidParameterValue",
        ),
        (
            "standard send with group",
            json!({"Command": "SendMessage", "QueueName": "q", "Body": "x", "GroupId": "g"}),
            "UnsupportedOperation",
        ),
        (
            "fifo send with delay",
            json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "x", "DelaySeconds": 0}),
            "UnsupportedOperation",
        ),
        (
            "fifo send without group",
            json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "x"}),
            "MissingParameter",
        ),
        (
            "fifo send without dedup id",
            json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "x", "GroupId": "g"}),
            "MissingParameter",
        ),
        (
            "bad group characters",
            json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "x", "GroupId": "g g"}),
            "InvalidParameterValue",
        ),
        (
            "receive too many",
            json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 11}),
            "InvalidParameterValue",
        ),
        (
            "delete with unissued handle",
            json!({"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": "r-1"}),
            "ReceiptHandleInvalid",
        ),
    ];
    let mut commands = vec![
        create("q", json!({})),
        create("f.fifo", json!({"Fifo": true})),
    ];
    commands.extend(cases.iter().map(|(_, command, _)| command.clone()));
    let responses = session(&commands);
    for ((name, _, expected), response) in cases.iter().zip(&responses[2..]) {
        assert_eq!(code(response), *expected, "{name}: {response}");
    }
}

#[test]
fn null_attributes_and_parameters_count_as_absent() {
    let responses = session(&[
        create("q", json!({"DelaySeconds": null, "Bogus": null})),
        json!({"Command": "CreateQueue", "QueueName": "r", "Attributes": null}),
        json!({"Command": "ListQueues", "Prefix": null}),
    ]);
    assert_eq!(responses[0], ok());
    assert_eq!(responses[1], ok());
    assert_eq!(responses[2], json!({"ok": true, "QueueNames": ["q", "r"]}));
}

#[test]
fn create_queue_is_idempotent_for_equal_attributes_only() {
    let responses = session(&[
        create("q", json!({"VisibilityTimeout": 10})),
        create("q", json!({"VisibilityTimeout": 10, "DelaySeconds": 0})),
        create("q", json!({})),
        create(
            "q",
            json!({"VisibilityTimeout": 10, "RetentionSeconds": 60}),
        ),
    ]);
    assert_eq!(responses[0], ok());
    assert_eq!(responses[1], ok());
    assert_eq!(code(&responses[2]), "QueueAlreadyExists");
    assert_eq!(code(&responses[3]), "QueueAlreadyExists");
}

#[test]
fn create_queue_compares_dead_letter_configuration() {
    let dead = json!({"DeadLetterQueue": "d", "MaxReceiveCount": 3});
    let responses = session(&[
        create("d", json!({})),
        create("q", dead.clone()),
        create("q", dead),
        create("q", json!({"DeadLetterQueue": "d", "MaxReceiveCount": 4})),
        create("q", json!({})),
        create("d", json!({"DeadLetterQueue": "q", "MaxReceiveCount": 1})),
    ]);
    let codes: Vec<&str> = responses
        .iter()
        .map(|r| {
            if r["ok"] == json!(true) {
                "ok"
            } else {
                code(r)
            }
        })
        .collect();
    assert_eq!(
        codes,
        [
            "ok",
            "ok",
            "ok",
            "QueueAlreadyExists",
            "QueueAlreadyExists",
            "QueueAlreadyExists"
        ]
    );
}

#[test]
fn get_queue_attributes_reports_defaults_and_set_values() {
    let responses = session(&[
        create("q", json!({})),
        attributes("q"),
        json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"DelaySeconds": 5}}),
        attributes("q"),
    ]);
    let defaults = json!({
        "VisibilityTimeout": 30, "DelaySeconds": 0, "RetentionSeconds": 345600,
        "Fifo": false, "ContentBasedDeduplication": false,
    });
    let counts = json!({"Visible": 0, "InFlight": 0, "Delayed": 0});
    assert_eq!(
        responses[1],
        json!({"ok": true, "Attributes": defaults, "Counts": counts})
    );
    let mut changed = defaults;
    changed["DelaySeconds"] = json!(5);
    assert_eq!(
        responses[3],
        json!({"ok": true, "Attributes": changed, "Counts": counts})
    );
}

#[test]
fn messages_are_redelivered_in_arrival_order_after_the_visibility_timeout() {
    let responses = session(&[
        create("q", json!({})),
        send("q", "a"),
        send("q", "b"),
        receive("q"),
        attributes("q"),
        tick(29),
        receive("q"),
        tick(1),
        receive("q"),
    ]);
    assert_eq!(responses[1], json!({"ok": true, "MessageId": "m-1"}));
    assert_eq!(responses[2], json!({"ok": true, "MessageId": "m-2"}));
    assert_eq!(
        responses[3],
        messages(vec![
            message(1, 1, "a", 1, 0, 0),
            message(2, 2, "b", 1, 0, 0)
        ])
    );
    assert_eq!(
        responses[4]["Counts"],
        json!({"Visible": 0, "InFlight": 2, "Delayed": 0})
    );
    assert_eq!(responses[6], messages(vec![]));
    assert_eq!(
        responses[8],
        messages(vec![
            message(1, 3, "a", 2, 0, 0),
            message(2, 4, "b", 2, 0, 0)
        ])
    );
}

#[test]
fn receive_honors_max_messages_and_a_per_call_visibility_timeout() {
    let responses = session(&[
        create("q", json!({})),
        send("q", "a"),
        send("q", "b"),
        json!({"Command": "ReceiveMessage", "QueueName": "q", "VisibilityTimeout": 5}),
        tick(5),
        receive("q"),
    ]);
    assert_eq!(responses[3], messages(vec![message(1, 1, "a", 1, 0, 0)]));
    assert_eq!(
        responses[5],
        messages(vec![
            message(1, 2, "a", 2, 0, 0),
            message(2, 3, "b", 1, 0, 5)
        ])
    );
}

#[test]
fn delay_hides_a_message_and_a_per_message_delay_overrides_the_queue() {
    let responses = session(&[
        create("q", json!({"DelaySeconds": 10})),
        send("q", "queue-delay"),
        json!({"Command": "SendMessage", "QueueName": "q", "Body": "now", "DelaySeconds": 0}),
        attributes("q"),
        receive("q"),
        tick(10),
        receive("q"),
    ]);
    assert_eq!(
        responses[3]["Counts"],
        json!({"Visible": 1, "InFlight": 0, "Delayed": 1})
    );
    assert_eq!(responses[4], messages(vec![message(2, 1, "now", 1, 0, 0)]));
    assert_eq!(
        responses[6],
        messages(vec![message(1, 2, "queue-delay", 1, 0, 10)])
    );
}

#[test]
fn retention_removes_messages_and_lowering_it_applies_at_once() {
    let counts = |response: &Value| response["Counts"].clone();
    let responses = session(&[
        create("short", json!({"RetentionSeconds": 60})),
        send("short", "a"),
        tick(59),
        attributes("short"),
        tick(1),
        attributes("short"),
        create("long", json!({})),
        send("long", "b"),
        tick(100),
        json!({"Command": "SetQueueAttributes", "QueueName": "long", "Attributes": {"RetentionSeconds": 100}}),
        json!({"Command": "SetQueueAttributes", "QueueName": "long", "Attributes": {"RetentionSeconds": 100000}}),
        attributes("long"),
    ]);
    assert_eq!(
        counts(&responses[3]),
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(
        counts(&responses[5]),
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(
        counts(&responses[11]),
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn delete_and_change_visibility_use_the_latest_handle_of_an_in_flight_message() {
    let delete = |handle: &str| json!({"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": handle});
    let change = |handle: &str, timeout: u64| json!({"Command": "ChangeMessageVisibility", "QueueName": "q", "ReceiptHandle": handle, "VisibilityTimeout": timeout});
    let responses = session(&[
        create("q", json!({"VisibilityTimeout": 10})),
        send("q", "a"),
        receive("q"), // r-1
        change("r-1", 100),
        tick(99),
        attributes("q"),
        tick(1),
        delete("r-1"), // timeout ended
        receive("q"),  // r-2
        change("r-2", 0),
        attributes("q"),
        delete("r-2"), // visible again
        receive("q"),  // r-3
        delete("r-1"), // replaced by a later delivery
        delete("r-3"),
        attributes("q"),
    ]);
    assert_eq!(responses[3], ok());
    assert_eq!(responses[5]["Counts"]["InFlight"], json!(1));
    assert_eq!(code(&responses[7]), "ReceiptHandleExpired");
    assert_eq!(responses[9], ok());
    assert_eq!(
        responses[10]["Counts"],
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(code(&responses[11]), "ReceiptHandleExpired");
    assert_eq!(responses[12], messages(vec![message(1, 3, "a", 3, 0, 0)]));
    assert_eq!(code(&responses[13]), "ReceiptHandleExpired");
    assert_eq!(responses[14], ok());
    assert_eq!(
        responses[15]["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn receipt_handles_are_exact_strings_and_scoped_to_their_queue() {
    let delete = |queue: &str, handle: &str| json!({"Command": "DeleteMessage", "QueueName": queue, "ReceiptHandle": handle});
    let responses = session(&[
        create("q", json!({})),
        create("other", json!({})),
        send("q", "a"),
        receive("q"),
        delete("q", "r-01"),
        delete("q", "R-1"),
        delete("q", "r-1 "),
        delete("q", "r-2"),
        delete("other", "r-1"),
        create("zero", json!({"VisibilityTimeout": 0})),
        send("zero", "a"),
        receive("zero"),
        delete("zero", "r-2"),
    ]);
    for index in [4, 5, 6, 7] {
        assert_eq!(
            code(&responses[index]),
            "ReceiptHandleInvalid",
            "line {index}"
        );
    }
    assert_eq!(code(&responses[8]), "ReceiptHandleExpired");
    assert_eq!(code(&responses[12]), "ReceiptHandleExpired");
}

#[test]
fn a_fifo_queue_delivers_one_message_per_group_in_order() {
    let delete = |handle: &str| json!({"Command": "DeleteMessage", "QueueName": "f.fifo", "ReceiptHandle": handle});
    let fifo_send = |body: &str, group: &str| json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": body, "GroupId": group, "DeduplicationId": body});
    let responses = session(&[
        create("f.fifo", json!({"Fifo": true})),
        fifo_send("a", "g1"),
        fifo_send("b", "g1"),
        fifo_send("c", "g2"),
        receive("f.fifo"),
        receive("f.fifo"),
        delete("r-1"),
        receive("f.fifo"),
    ]);
    assert_eq!(
        responses[1],
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
    assert_eq!(
        responses[3],
        json!({"ok": true, "MessageId": "m-3", "SequenceNumber": 3})
    );
    assert_eq!(
        responses[4],
        messages(vec![
            fifo_message(message(1, 1, "a", 1, 0, 0), "g1", 1),
            fifo_message(message(3, 2, "c", 1, 0, 0), "g2", 3),
        ])
    );
    assert_eq!(responses[5], messages(vec![]));
    assert_eq!(
        responses[7],
        messages(vec![fifo_message(message(2, 3, "b", 1, 0, 0), "g1", 2)])
    );
}

#[test]
fn a_delayed_or_in_flight_head_blocks_its_group_but_a_later_group_is_served() {
    let fifo_send = |body: &str, group: &str| json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": body, "GroupId": group, "DeduplicationId": body});
    let responses = session(&[
        create("f.fifo", json!({"Fifo": true, "DelaySeconds": 5})),
        fifo_send("a", "g1"),
        tick(3),
        json!({"Command": "SetQueueAttributes", "QueueName": "f.fifo", "Attributes": {"DelaySeconds": 0}}),
        fifo_send("b", "g1"),
        fifo_send("c", "g2"),
        receive("f.fifo"),
        tick(2),
        receive("f.fifo"),
    ]);
    assert_eq!(
        responses[6],
        messages(vec![fifo_message(message(3, 1, "c", 1, 3, 3), "g2", 3)])
    );
    assert_eq!(
        responses[8],
        messages(vec![fifo_message(message(1, 2, "a", 1, 0, 5), "g1", 1)])
    );
}

#[test]
fn fifo_deduplication_window_is_300_seconds_and_is_not_extended() {
    let fifo_send = |body: &str, id: &str| json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": body, "GroupId": "g", "DeduplicationId": id});
    let responses = session(&[
        create("f.fifo", json!({"Fifo": true})),
        fifo_send("a", "x"),
        tick(299),
        fifo_send("different body", "x"),
        tick(1),
        fifo_send("b", "x"),
        tick(299),
        fifo_send("c", "x"),
        fifo_send("d", "y"),
    ]);
    assert_eq!(
        responses[3],
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
    assert_eq!(
        responses[5],
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2})
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
fn content_based_deduplication_uses_the_body_and_ignores_the_group() {
    let fifo_send = |body: &str, group: &str| json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": body, "GroupId": group});
    let responses = session(&[
        create(
            "f.fifo",
            json!({"Fifo": true, "ContentBasedDeduplication": true}),
        ),
        fifo_send("same", "g1"),
        fifo_send("same", "g2"),
        json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "other", "GroupId": "g1", "DeduplicationId": "same"}),
        fifo_send("same\u{e9}", "g1"),
    ]);
    assert_eq!(
        responses[2],
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
    assert_eq!(
        responses[3],
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
    assert_eq!(
        responses[4],
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2})
    );
}

#[test]
fn deduplication_ids_outlive_deleted_and_purged_messages() {
    let fifo_send = |id: &str| json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "x", "GroupId": "g", "DeduplicationId": id});
    let responses = session(&[
        create("f.fifo", json!({"Fifo": true})),
        fifo_send("x"),
        receive("f.fifo"),
        json!({"Command": "DeleteMessage", "QueueName": "f.fifo", "ReceiptHandle": "r-1"}),
        fifo_send("x"),
        json!({"Command": "PurgeQueue", "QueueName": "f.fifo"}),
        fifo_send("x"),
        fifo_send("z"),
    ]);
    assert_eq!(
        responses[4],
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
    assert_eq!(
        responses[6],
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
    assert_eq!(
        responses[7],
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2})
    );
}

#[test]
fn a_failed_send_does_not_use_up_a_message_id() {
    let responses = session(&[create("q", json!({})), send("q", ""), send("q", "a")]);
    assert_eq!(responses[2], json!({"ok": true, "MessageId": "m-1"}));
}

#[test]
fn messages_over_the_receive_limit_move_to_the_dead_letter_queue() {
    let responses = session(&[
        create("d", json!({})),
        create(
            "q",
            json!({"DeadLetterQueue": "d", "MaxReceiveCount": 2, "VisibilityTimeout": 0}),
        ),
        send("q", "a"),
        receive("q"),
        receive("q"),
        receive("q"),
        attributes("q"),
        receive("d"),
    ]);
    assert_eq!(responses[3], messages(vec![message(1, 1, "a", 1, 0, 0)]));
    assert_eq!(responses[4], messages(vec![message(1, 2, "a", 2, 0, 0)]));
    assert_eq!(responses[5], messages(vec![]));
    assert_eq!(
        responses[6]["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(responses[7], messages(vec![message(1, 3, "a", 1, 0, 0)]));
}

#[test]
fn a_dead_letter_move_does_not_count_toward_max_messages_and_keeps_going() {
    let responses = session(&[
        create("d", json!({})),
        create(
            "q",
            json!({"DeadLetterQueue": "d", "MaxReceiveCount": 1, "VisibilityTimeout": 0}),
        ),
        send("q", "old"),
        receive("q"),
        send("q", "new"),
        json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 1}),
        receive("d"),
    ]);
    assert_eq!(responses[5], messages(vec![message(2, 2, "new", 1, 0, 0)]));
    assert_eq!(responses[6], messages(vec![message(1, 3, "old", 1, 0, 0)]));
}

#[test]
fn a_dead_lettered_message_already_too_old_for_the_target_is_dropped() {
    let responses = session(&[
        create("d", json!({"RetentionSeconds": 60})),
        create(
            "q",
            json!({"DeadLetterQueue": "d", "MaxReceiveCount": 1, "VisibilityTimeout": 0}),
        ),
        send("q", "a"),
        receive("q"),
        tick(100),
        receive("q"),
        attributes("d"),
    ]);
    assert_eq!(responses[5], messages(vec![]));
    assert_eq!(
        responses[6]["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn a_fifo_dead_letter_move_takes_the_next_sequence_number_and_unblocks_the_group() {
    let fifo_send = |queue: &str, body: &str| json!({"Command": "SendMessage", "QueueName": queue, "Body": body, "GroupId": "g", "DeduplicationId": body});
    let responses = session(&[
        create("d.fifo", json!({"Fifo": true})),
        fifo_send("d.fifo", "first-in-d"),
        create(
            "q.fifo",
            json!({"Fifo": true, "DeadLetterQueue": "d.fifo", "MaxReceiveCount": 1, "VisibilityTimeout": 0}),
        ),
        fifo_send("q.fifo", "a"),
        fifo_send("q.fifo", "b"),
        json!({"Command": "ReceiveMessage", "QueueName": "q.fifo"}),
        json!({"Command": "ReceiveMessage", "QueueName": "q.fifo"}),
        receive("d.fifo"),
        json!({"Command": "DeleteMessage", "QueueName": "d.fifo", "ReceiptHandle": "r-3"}),
        receive("d.fifo"),
    ]);
    assert_eq!(
        responses[5],
        messages(vec![fifo_message(message(2, 1, "a", 1, 0, 0), "g", 1)])
    );
    assert_eq!(
        responses[6],
        messages(vec![fifo_message(message(3, 2, "b", 1, 0, 0), "g", 2)])
    );
    // One message per group per receive: the moved message waits behind the older one.
    assert_eq!(
        responses[7],
        messages(vec![fifo_message(
            message(1, 3, "first-in-d", 1, 0, 0),
            "g",
            1
        )])
    );
    assert_eq!(
        responses[9],
        messages(vec![fifo_message(message(2, 4, "a", 1, 0, 0), "g", 2)])
    );
}

#[test]
fn deleting_a_queue_clears_dead_letter_configurations_that_name_it() {
    let responses = session(&[
        create("d", json!({})),
        create("q", json!({"DeadLetterQueue": "d", "MaxReceiveCount": 5})),
        json!({"Command": "DeleteQueue", "QueueName": "d"}),
        attributes("q"),
        create("d", json!({})),
        attributes("q"),
        json!({"Command": "DeleteQueue", "QueueName": "d"}),
    ]);
    let without_dead_letter = json!({
        "VisibilityTimeout": 30, "DelaySeconds": 0, "RetentionSeconds": 345600,
        "Fifo": false, "ContentBasedDeduplication": false,
    });
    assert_eq!(responses[3]["Attributes"], without_dead_letter);
    assert_eq!(responses[5]["Attributes"], without_dead_letter);
}

#[test]
fn a_recreated_queue_shares_nothing_with_the_deleted_one() {
    let responses = session(&[
        create("q", json!({})),
        send("q", "a"),
        json!({"Command": "DeleteQueue", "QueueName": "q"}),
        create("q", json!({})),
        attributes("q"),
        json!({"Command": "DeleteQueue", "QueueName": "q"}),
    ]);
    assert_eq!(
        responses[4]["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(responses[5], ok());
}

#[test]
fn list_queues_sorts_by_byte_value_and_filters_by_prefix() {
    let mut commands: Vec<Value> = ["b", "a", "B", "_x", "a-1", "a1"]
        .into_iter()
        .map(|name| create(name, json!({})))
        .collect();
    commands.push(create("a.fifo", json!({"Fifo": true})));
    commands.push(json!({"Command": "ListQueues"}));
    commands.push(json!({"Command": "ListQueues", "Prefix": "a"}));
    commands.push(json!({"Command": "ListQueues", "Prefix": "zz"}));
    let responses = session(&commands);
    assert_eq!(
        responses[7],
        json!({"ok": true, "QueueNames": ["B", "_x", "a", "a-1", "a.fifo", "a1", "b"]})
    );
    assert_eq!(
        responses[8],
        json!({"ok": true, "QueueNames": ["a", "a-1", "a.fifo", "a1"]})
    );
    assert_eq!(responses[9], json!({"ok": true, "QueueNames": []}));
}

#[test]
fn purge_removes_every_state_but_keeps_sequence_numbers() {
    let fifo_send = |id: &str| json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "x", "GroupId": "g", "DeduplicationId": id});
    let responses = session(&[
        create("q", json!({})),
        send("q", "a"),
        receive("q"),
        send("q", "b"),
        json!({"Command": "PurgeQueue", "QueueName": "q"}),
        attributes("q"),
        create("f.fifo", json!({"Fifo": true})),
        fifo_send("1"),
        json!({"Command": "PurgeQueue", "QueueName": "f.fifo"}),
        fifo_send("2"),
    ]);
    assert_eq!(
        responses[5]["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(
        responses[9],
        json!({"ok": true, "MessageId": "m-4", "SequenceNumber": 2})
    );
}

#[test]
fn a_new_dead_letter_configuration_applies_from_the_next_receive() {
    let responses = session(&[
        create("d", json!({})),
        create("q", json!({"VisibilityTimeout": 0})),
        send("q", "a"),
        receive("q"),
        json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"DeadLetterQueue": "d", "MaxReceiveCount": 1}}),
        receive("q"),
        receive("d"),
    ]);
    assert_eq!(responses[5], messages(vec![]));
    assert_eq!(responses[6], messages(vec![message(1, 2, "a", 1, 0, 0)]));
}

#[test]
fn the_queue_count_is_limited_to_1000_and_an_equal_create_still_succeeds() {
    let mut commands: Vec<Value> = (0..1000)
        .map(|n| create(&format!("q{n}"), json!({})))
        .collect();
    commands.push(create("extra", json!({})));
    commands.push(create("q0", json!({})));
    let responses = session(&commands);
    assert!(responses[..1000].iter().all(|r| *r == ok()));
    assert_eq!(code(&responses[1000]), "LimitExceeded");
    assert_eq!(responses[1001], ok());
}

#[test]
fn a_queue_holds_at_most_100000_messages_counting_every_state() {
    let mut commands = vec![create("q", json!({}))];
    commands.extend((0..100_000).map(|_| send("q", "x")));
    commands.push(send("q", "x"));
    commands.push(receive("q"));
    commands.push(send("q", "x"));
    let responses = session(&commands);
    assert_eq!(
        responses[100_000],
        json!({"ok": true, "MessageId": "m-100000"})
    );
    assert_eq!(code(&responses[100_001]), "LimitExceeded");
    assert_eq!(code(&responses[100_003]), "LimitExceeded");
}
