//! Drives the service through its line protocol and compares whole responses.

use serde_json::{Value, json};

/// Runs `input` through the service and returns one response per line, with the free-text
/// error message removed so that errors compare by code.
fn exchange(input: &[u8]) -> Vec<Value> {
    let mut output = Vec::new();
    miniq::run(input, &mut output).expect("in-memory I/O does not fail");
    std::str::from_utf8(&output)
        .expect("responses are UTF-8")
        .lines()
        .map(|line| {
            let mut response: Value = serde_json::from_str(line).expect("each response is JSON");
            if let Some(error) = response.get_mut("error").and_then(Value::as_object_mut) {
                error.remove("message");
            }
            response
        })
        .collect()
}

fn session(commands: &[Value]) -> Vec<Value> {
    let input: String = commands
        .iter()
        .map(|command| format!("{command}\n"))
        .collect();
    exchange(input.as_bytes())
}

fn ok() -> Value {
    json!({"ok": true})
}

fn fail(code: &str) -> Value {
    json!({"ok": false, "error": {"code": code}})
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

fn only_counts(response: &Value) -> &Value {
    &response["Counts"]
}

#[test]
fn every_line_gets_one_answer_even_when_malformed() {
    let responses = exchange(
        b"\n\xff\n{\"Command\":\"Tick\",\"Seconds\":null}\n{\"Command\":\"Tick\",\"Seconds\":2}",
    );
    assert_eq!(
        responses,
        [
            fail("InvalidRequest"),
            fail("InvalidRequest"),
            fail("MissingParameter"),
            json!({"ok": true, "Now": 2}),
        ]
    );
}

#[test]
fn failures_are_ranked_by_check_order() {
    let cases = [
        (r#"{"Command":"Nope"}"#, "InvalidRequest"),
        (r#"{"Command":"CreateQueue"}"#, "MissingParameter"),
        (
            r#"{"Command":"CreateQueue","QueueName":"bad name","Attributes":5}"#,
            "QueueNameInvalid",
        ),
        (
            r#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"Fifo":true}}"#,
            "QueueNameInvalid",
        ),
        (
            r#"{"Command":"CreateQueue","QueueName":"q.fifo"}"#,
            "QueueNameInvalid",
        ),
        (
            r#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"ContentBasedDeduplication":true}}"#,
            "InvalidParameterValue",
        ),
        (
            r#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"DeadLetterQueue":"x"}}"#,
            "InvalidParameterValue",
        ),
        (
            r#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"DeadLetterQueue":"x","MaxReceiveCount":1}}"#,
            "QueueDoesNotExist",
        ),
        (
            r#"{"Command":"SendMessage","QueueName":"ghost","Body":""}"#,
            "InvalidParameterValue",
        ),
        (
            r#"{"Command":"SendMessage","QueueName":"ghost","Body":"x"}"#,
            "QueueDoesNotExist",
        ),
        (
            r#"{"Command":"GetQueueAttributes","QueueName":""}"#,
            "QueueDoesNotExist",
        ),
        (
            r#"{"Command":"SetQueueAttributes","QueueName":"ghost","Attributes":{}}"#,
            "QueueDoesNotExist",
        ),
        (
            r#"{"Command":"ReceiveMessage","QueueName":"q","MaxMessages":11}"#,
            "InvalidParameterValue",
        ),
        (
            r#"{"Command":"DeleteMessage","QueueName":"ghost","ReceiptHandle":"r-1"}"#,
            "QueueDoesNotExist",
        ),
        (
            r#"{"Command":"Tick","Seconds":1.0}"#,
            "InvalidParameterValue",
        ),
    ];
    for (line, code) in cases {
        assert_eq!(exchange(line.as_bytes()), [fail(code)], "{line}");
    }
}

#[test]
fn standard_queue_delivers_in_arrival_order_and_redelivers_after_the_timeout() {
    let responses = session(&[
        create("q", json!({})),
        send("q", "a"),
        send("q", "b"),
        json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 5}),
        receive("q"),
        tick(30),
        receive("q"),
    ]);
    let delivery = |id: &str, handle: &str, body: &str, count: u32| json!({"MessageId": id, "ReceiptHandle": handle, "Body": body, "ReceiveCount": count, "SentAt": 0, "FirstReceivedAt": 0});
    assert_eq!(
        responses,
        [
            ok(),
            json!({"ok": true, "MessageId": "m-1"}),
            json!({"ok": true, "MessageId": "m-2"}),
            json!({"ok": true, "Messages": [delivery("m-1", "r-1", "a", 1), delivery("m-2", "r-2", "b", 1)]}),
            json!({"ok": true, "Messages": []}),
            json!({"ok": true, "Now": 30}),
            json!({"ok": true, "Messages": [delivery("m-1", "r-3", "a", 2)]}),
        ]
    );
}

#[test]
fn delayed_messages_become_visible_when_their_delay_ends() {
    let responses = session(&[
        create("q", json!({"DelaySeconds": 10})),
        send("q", "a"),
        json!({"Command": "SendMessage", "QueueName": "q", "Body": "b", "DelaySeconds": 0}),
        counts("q"),
        receive("q"),
        tick(10),
        receive("q"),
    ]);
    assert_eq!(
        responses[3]["Counts"],
        json!({"Visible": 1, "InFlight": 0, "Delayed": 1})
    );
    assert_eq!(responses[4]["Messages"][0]["MessageId"], "m-2");
    assert_eq!(responses[6]["Messages"][0]["MessageId"], "m-1");
    assert_eq!(responses[6]["Messages"][0]["SentAt"], 0);
}

#[test]
fn creating_an_existing_queue_succeeds_only_with_equal_attributes() {
    assert_eq!(
        session(&[
            create("q", json!({})),
            create("q", json!({"VisibilityTimeout": 30, "Fifo": false})),
            create("q", json!({"VisibilityTimeout": 31})),
        ]),
        [ok(), ok(), fail("QueueAlreadyExists")]
    );
}

#[test]
fn set_queue_attributes_checks_its_rules_in_order() {
    let set = |attributes: Value| json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": attributes});
    let responses = session(&[
        create("q", json!({})),
        create("f.fifo", json!({"Fifo": true})),
        create("d", json!({})),
        set(json!({"Fifo": false})),
        set(json!({"ContentBasedDeduplication": true})),
        set(json!({"DeadLetterQueue": "q", "MaxReceiveCount": 3})),
        set(json!({"DeadLetterQueue": "f.fifo", "MaxReceiveCount": 3})),
        set(json!({"MaxReceiveCount": 3})),
        set(json!({"DeadLetterQueue": "nope", "MaxReceiveCount": 3})),
        set(json!({"VisibilityTimeout": 5, "DeadLetterQueue": "d", "MaxReceiveCount": 3})),
        counts("q"),
    ]);
    assert_eq!(
        responses[3..],
        [
            fail("UnsupportedOperation"),
            fail("InvalidParameterValue"),
            fail("InvalidParameterValue"),
            fail("InvalidParameterValue"),
            fail("InvalidParameterValue"),
            fail("QueueDoesNotExist"),
            ok(),
            json!({
                "ok": true,
                "Attributes": {
                    "VisibilityTimeout": 5, "DelaySeconds": 0, "RetentionSeconds": 345600,
                    "Fifo": false, "ContentBasedDeduplication": false,
                    "DeadLetterQueue": "d", "MaxReceiveCount": 3
                },
                "Counts": {"Visible": 0, "InFlight": 0, "Delayed": 0}
            }),
        ]
    );
}

#[test]
fn list_queues_sorts_by_byte_value_and_filters_by_prefix() {
    let list = |prefix: Value| json!({"Command": "ListQueues", "Prefix": prefix});
    let responses = session(&[
        create("b", json!({})),
        create("a", json!({})),
        create("A", json!({})),
        create("_x", json!({})),
        create("a-1", json!({})),
        create("a.fifo", json!({"Fifo": true})),
        create("1", json!({})),
        list(Value::Null),
        list(json!("a")),
        list(json!("zzz")),
    ]);
    assert_eq!(
        responses[7..],
        [
            json!({"ok": true, "QueueNames": ["1", "A", "_x", "a", "a-1", "a.fifo", "b"]}),
            json!({"ok": true, "QueueNames": ["a", "a-1", "a.fifo"]}),
            json!({"ok": true, "QueueNames": []}),
        ]
    );
}

#[test]
fn fifo_queue_blocks_a_group_until_its_earlier_message_is_gone() {
    let fifo_send = |body: &str, group: &str, dedup: &str| json!({"Command": "SendMessage", "QueueName": "g.fifo", "Body": body, "GroupId": group, "DeduplicationId": dedup});
    let responses = session(&[
        create("g.fifo", json!({"Fifo": true})),
        fifo_send("a", "A", "d1"),
        fifo_send("b", "A", "d2"),
        fifo_send("c", "B", "d3"),
        fifo_send("zzz", "A", "d1"),
        json!({"Command": "SendMessage", "QueueName": "g.fifo", "Body": "x", "DeduplicationId": "d9"}),
        json!({"Command": "SendMessage", "QueueName": "g.fifo", "Body": "x", "GroupId": "A", "DeduplicationId": "d9", "DelaySeconds": 0}),
        json!({"Command": "ReceiveMessage", "QueueName": "g.fifo", "MaxMessages": 10}),
        json!({"Command": "DeleteMessage", "QueueName": "g.fifo", "ReceiptHandle": "r-1"}),
        json!({"Command": "ReceiveMessage", "QueueName": "g.fifo", "MaxMessages": 10}),
    ]);
    let delivery = |id: &str, handle: &str, body: &str, group: &str, sequence: u32| json!({"MessageId": id, "ReceiptHandle": handle, "Body": body, "ReceiveCount": 1, "SentAt": 0, "FirstReceivedAt": 0, "GroupId": group, "SequenceNumber": sequence});
    assert_eq!(
        responses[1..],
        [
            json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1}),
            json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2}),
            json!({"ok": true, "MessageId": "m-3", "SequenceNumber": 3}),
            json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1}),
            fail("MissingParameter"),
            fail("UnsupportedOperation"),
            json!({"ok": true, "Messages": [delivery("m-1", "r-1", "a", "A", 1), delivery("m-3", "r-2", "c", "B", 3)]}),
            ok(),
            json!({"ok": true, "Messages": [delivery("m-2", "r-3", "b", "A", 2)]}),
        ]
    );
}

#[test]
fn deduplication_window_closes_after_300_seconds_and_outlives_the_message() {
    let fifo_send = json!({"Command": "SendMessage", "QueueName": "g.fifo", "Body": "a", "GroupId": "A", "DeduplicationId": "d"});
    let responses = session(&[
        create("g.fifo", json!({"Fifo": true})),
        fifo_send.clone(),
        json!({"Command": "PurgeQueue", "QueueName": "g.fifo"}),
        tick(299),
        fifo_send.clone(),
        tick(1),
        fifo_send.clone(),
        fifo_send,
    ]);
    assert_eq!(
        responses[4..],
        [
            json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1}),
            json!({"ok": true, "Now": 300}),
            json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2}),
            json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2}),
        ]
    );
}

#[test]
fn content_based_deduplication_uses_the_body_whatever_the_group() {
    let fifo_send = |group: &str| json!({"Command": "SendMessage", "QueueName": "c.fifo", "Body": "same", "GroupId": group});
    assert_eq!(
        session(&[
            create(
                "c.fifo",
                json!({"Fifo": true, "ContentBasedDeduplication": true})
            ),
            fifo_send("A"),
            fifo_send("B"),
        ])[1..],
        [
            json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1}),
            json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1}),
        ]
    );
}

#[test]
fn exhausted_messages_move_to_the_dead_letter_queue_with_a_fresh_history() {
    let responses = session(&[
        create("dead", json!({})),
        create(
            "q",
            json!({"DeadLetterQueue": "dead", "MaxReceiveCount": 2, "VisibilityTimeout": 0}),
        ),
        send("q", "a"),
        receive("q"),
        receive("q"),
        tick(7),
        receive("q"),
        receive("dead"),
        json!({"Command": "DeleteQueue", "QueueName": "dead"}),
        counts("q"),
        json!({"Command": "DeleteQueue", "QueueName": "dead"}),
    ]);
    assert_eq!(responses[6], json!({"ok": true, "Messages": []}));
    assert_eq!(
        responses[7],
        json!({"ok": true, "Messages": [{
            "MessageId": "m-1", "ReceiptHandle": "r-3", "Body": "a",
            "ReceiveCount": 1, "SentAt": 0, "FirstReceivedAt": 7
        }]})
    );
    assert_eq!(
        responses[9]["Attributes"],
        json!({
            "VisibilityTimeout": 0, "DelaySeconds": 0, "RetentionSeconds": 345600,
            "Fifo": false, "ContentBasedDeduplication": false
        })
    );
    assert_eq!(responses[10], fail("QueueDoesNotExist"));
}

#[test]
fn receipt_handles_are_invalid_when_never_issued_and_expired_when_not_usable() {
    let delete = |queue: &str, handle: &str| json!({"Command": "DeleteMessage", "QueueName": queue, "ReceiptHandle": handle});
    let responses = session(&[
        create("q", json!({})),
        create("other", json!({})),
        send("q", "a"),
        receive("q"),
        delete("q", "r-9"),
        delete("q", "r-01"),
        delete("other", "r-1"),
        json!({"Command": "ChangeMessageVisibility", "QueueName": "q", "ReceiptHandle": "r-1", "VisibilityTimeout": 100}),
        tick(31),
        receive("q"),
        tick(69),
        receive("q"),
        delete("q", "r-1"),
        delete("q", "r-2"),
        delete("q", "r-2"),
    ]);
    assert_eq!(
        responses[4..],
        [
            fail("ReceiptHandleInvalid"),
            fail("ReceiptHandleInvalid"),
            fail("ReceiptHandleExpired"),
            ok(),
            json!({"ok": true, "Now": 31}),
            json!({"ok": true, "Messages": []}),
            json!({"ok": true, "Now": 100}),
            json!({"ok": true, "Messages": [{
                "MessageId": "m-1", "ReceiptHandle": "r-2", "Body": "a",
                "ReceiveCount": 2, "SentAt": 0, "FirstReceivedAt": 0
            }]}),
            fail("ReceiptHandleExpired"),
            ok(),
            fail("ReceiptHandleExpired"),
        ]
    );
}

#[test]
fn a_zero_visibility_timeout_makes_the_handle_unusable() {
    let responses = session(&[
        create("q", json!({"VisibilityTimeout": 0})),
        send("q", "a"),
        receive("q"),
        json!({"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": "r-1"}),
    ]);
    assert_eq!(responses[3], fail("ReceiptHandleExpired"));
}

#[test]
fn retention_applies_at_once_and_is_not_undone_by_raising_it() {
    let responses = session(&[
        create("q", json!({"RetentionSeconds": 60})),
        send("q", "a"),
        tick(59),
        counts("q"),
        tick(1),
        counts("q"),
        create("r", json!({})),
        json!({"Command": "SendMessage", "QueueName": "r", "Body": "a"}),
        tick(100),
        json!({"Command": "SetQueueAttributes", "QueueName": "r", "Attributes": {"RetentionSeconds": 60}}),
        json!({"Command": "SetQueueAttributes", "QueueName": "r", "Attributes": {"RetentionSeconds": 1000}}),
        counts("r"),
    ]);
    assert_eq!(
        only_counts(&responses[3]),
        &json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(
        only_counts(&responses[5]),
        &json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(
        only_counts(&responses[11]),
        &json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn purge_keeps_sequence_numbers_and_deduplication() {
    let fifo_send = |dedup: &str| json!({"Command": "SendMessage", "QueueName": "p.fifo", "Body": "a", "GroupId": "g", "DeduplicationId": dedup});
    let responses = session(&[
        create("p.fifo", json!({"Fifo": true})),
        fifo_send("d"),
        json!({"Command": "PurgeQueue", "QueueName": "p.fifo"}),
        fifo_send("d"),
        fifo_send("e"),
    ]);
    assert_eq!(
        responses[3..],
        [
            json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1}),
            json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2}),
        ]
    );
}

#[test]
fn queue_and_message_limits_are_enforced_and_fast() {
    let mut input = String::new();
    for n in 0..1001 {
        input.push_str(&format!(
            "{{\"Command\":\"CreateQueue\",\"QueueName\":\"q{n}\"}}\n"
        ));
    }
    input.push_str("{\"Command\":\"CreateQueue\",\"QueueName\":\"q0\"}\n");
    let responses = exchange(input.as_bytes());
    assert_eq!(responses[999], ok());
    assert_eq!(responses[1000], fail("LimitExceeded"));
    assert_eq!(responses[1001], ok());

    let mut input = String::from("{\"Command\":\"CreateQueue\",\"QueueName\":\"q\"}\n");
    for _ in 0..100_001 {
        input.push_str("{\"Command\":\"SendMessage\",\"QueueName\":\"q\",\"Body\":\"x\"}\n");
    }
    let responses = exchange(input.as_bytes());
    assert_eq!(
        responses[100_000],
        json!({"ok": true, "MessageId": "m-100000"})
    );
    assert_eq!(responses[100_001], fail("LimitExceeded"));
}
