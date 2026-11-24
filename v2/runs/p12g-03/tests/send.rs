mod common;

use common::{error_code, exec, session};
use serde_json::{Value, json};

fn create(name: &str, attributes: Value) -> Value {
    json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attributes})
}

fn send(queue: &str, body: &str) -> Value {
    json!({"Command": "SendMessage", "QueueName": queue, "Body": body})
}

fn fifo_send(queue: &str, body: &str, group: &str, dedup: &str) -> Value {
    json!({"Command": "SendMessage", "QueueName": queue, "Body": body, "GroupId": group, "DeduplicationId": dedup})
}

fn tick(seconds: u32) -> Value {
    json!({"Command": "Tick", "Seconds": seconds})
}

#[test]
fn standard_sends_get_consecutive_ids_across_queues() {
    let responses = exec(&[
        create("a", json!({})),
        create("b", json!({})),
        send("a", "x"),
        send("b", "y"),
        send("missing", "z"),
        send("a", ""),
        send("a", "w"),
    ]);
    assert_eq!(responses[2], json!({"ok": true, "MessageId": "m-1"}));
    assert_eq!(responses[3], json!({"ok": true, "MessageId": "m-2"}));
    assert_eq!(error_code(&responses[4]), Some("QueueDoesNotExist"));
    assert_eq!(error_code(&responses[5]), Some("InvalidParameterValue"));
    assert_eq!(responses[6], json!({"ok": true, "MessageId": "m-3"}));
}

#[test]
fn send_validates_parameters_before_looking_up_the_queue() {
    let two_byte_chars_ok = "\u{e9}".repeat(512);
    let two_byte_chars_too_long = "\u{e9}".repeat(513);
    let cases: Vec<(Value, &str)> = vec![
        (
            json!({"Command": "SendMessage", "QueueName": "nope"}),
            "MissingParameter",
        ),
        (
            json!({"Command": "SendMessage", "Body": "x"}),
            "MissingParameter",
        ),
        (
            json!({"Command": "SendMessage", "QueueName": 1, "Body": null}),
            "MissingParameter",
        ),
        (
            json!({"Command": "SendMessage", "QueueName": "nope", "Body": 5}),
            "InvalidParameterValue",
        ),
        (
            json!({"Command": "SendMessage", "QueueName": "nope", "Body": "a".repeat(1025)}),
            "InvalidParameterValue",
        ),
        (
            json!({"Command": "SendMessage", "QueueName": "nope", "Body": two_byte_chars_too_long}),
            "InvalidParameterValue",
        ),
        (
            json!({"Command": "SendMessage", "QueueName": "nope", "Body": "x", "DelaySeconds": 901}),
            "InvalidParameterValue",
        ),
        (
            json!({"Command": "SendMessage", "QueueName": "nope", "Body": "x", "GroupId": "a b"}),
            "InvalidParameterValue",
        ),
        (
            json!({"Command": "SendMessage", "QueueName": "nope", "Body": "x", "GroupId": "a".repeat(129)}),
            "InvalidParameterValue",
        ),
        (
            json!({"Command": "SendMessage", "QueueName": "nope", "Body": "x", "DeduplicationId": ""}),
            "InvalidParameterValue",
        ),
        (
            json!({"Command": "SendMessage", "QueueName": "nope", "Body": "x"}),
            "QueueDoesNotExist",
        ),
        (
            json!({"Command": "SendMessage", "QueueName": "nope", "Body": two_byte_chars_ok}),
            "QueueDoesNotExist",
        ),
        (
            json!({"Command": "SendMessage", "QueueName": "nope", "Body": "a".repeat(1024)}),
            "QueueDoesNotExist",
        ),
    ];
    for (command, code) in cases {
        let response = &exec(std::slice::from_ref(&command))[0];
        assert_eq!(error_code(response), Some(code), "{command}");
    }
}

#[test]
fn standard_queue_rejects_fifo_parameters_and_fifo_queue_rejects_delay() {
    let responses = exec(&[
        create("s", json!({})),
        create("f.fifo", json!({"Fifo": true})),
        json!({"Command": "SendMessage", "QueueName": "s", "Body": "x", "GroupId": "g"}),
        json!({"Command": "SendMessage", "QueueName": "s", "Body": "x", "DeduplicationId": "d"}),
        json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "x", "DelaySeconds": 0}),
        json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "x", "DelaySeconds": 0, "GroupId": "g"}),
    ]);
    let codes: Vec<_> = responses[2..].iter().map(error_code).collect();
    assert_eq!(codes, [Some("UnsupportedOperation"); 4]);
}

#[test]
fn fifo_send_needs_group_and_a_deduplication_source() {
    let responses = exec(&[
        create("f.fifo", json!({"Fifo": true})),
        create(
            "c.fifo",
            json!({"Fifo": true, "ContentBasedDeduplication": true}),
        ),
        json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "x", "DeduplicationId": "d"}),
        json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "x", "GroupId": "g"}),
        json!({"Command": "SendMessage", "QueueName": "c.fifo", "Body": "x"}),
        json!({"Command": "SendMessage", "QueueName": "c.fifo", "Body": "x", "GroupId": "g"}),
    ]);
    assert_eq!(error_code(&responses[2]), Some("MissingParameter"));
    assert_eq!(error_code(&responses[3]), Some("MissingParameter"));
    assert_eq!(error_code(&responses[4]), Some("MissingParameter"));
    assert_eq!(
        responses[5],
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
}

#[test]
fn fifo_duplicates_return_the_original_within_300_seconds_only() {
    let responses = exec(&[
        create("f.fifo", json!({"Fifo": true})),
        fifo_send("f.fifo", "a", "g1", "d"),
        tick(299),
        fifo_send("f.fifo", "other body", "g2", "d"),
        tick(1),
        fifo_send("f.fifo", "again", "g1", "d"),
        tick(299),
        fifo_send("f.fifo", "again", "g1", "d"),
        fifo_send("f.fifo", "fresh", "g1", "e"),
    ]);
    assert_eq!(
        responses[1],
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
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
fn content_based_deduplication_uses_the_body_unless_an_id_is_given_in_one_id_space() {
    let responses = exec(&[
        create(
            "c.fifo",
            json!({"Fifo": true, "ContentBasedDeduplication": true}),
        ),
        json!({"Command": "SendMessage", "QueueName": "c.fifo", "Body": "same", "GroupId": "g"}),
        json!({"Command": "SendMessage", "QueueName": "c.fifo", "Body": "same", "GroupId": "h"}),
        fifo_send("c.fifo", "same", "g", "explicit"),
        json!({"Command": "SendMessage", "QueueName": "c.fifo", "Body": "explicit", "GroupId": "g"}),
    ]);
    assert_eq!(responses[1]["MessageId"], json!("m-1"));
    assert_eq!(responses[2]["MessageId"], json!("m-1"));
    assert_eq!(responses[3]["MessageId"], json!("m-2"));
    assert_eq!(
        responses[4]["MessageId"],
        json!("m-2"),
        "a body and an explicit id with the same text share one id space"
    );
}

#[test]
fn deduplication_ids_are_per_queue() {
    let responses = exec(&[
        create("a.fifo", json!({"Fifo": true})),
        create("b.fifo", json!({"Fifo": true})),
        fifo_send("a.fifo", "x", "g", "d"),
        fifo_send("b.fifo", "x", "g", "d"),
    ]);
    assert_eq!(
        responses[2],
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
    assert_eq!(
        responses[3],
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 1})
    );
}

#[test]
fn a_full_queue_refuses_the_next_send_in_reasonable_time() {
    let mut input = String::from("{\"Command\":\"CreateQueue\",\"QueueName\":\"big\"}\n");
    for _ in 0..100_000 {
        input.push_str("{\"Command\":\"SendMessage\",\"QueueName\":\"big\",\"Body\":\"x\"}\n");
    }
    input.push_str("{\"Command\":\"SendMessage\",\"QueueName\":\"big\",\"Body\":\"x\"}\n");
    let responses = session(&input);
    assert_eq!(responses.len(), 100_002);
    assert_eq!(
        responses[100_000],
        json!({"ok": true, "MessageId": "m-100000"})
    );
    assert_eq!(error_code(&responses[100_001]), Some("LimitExceeded"));
}
