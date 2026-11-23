mod common;

use common::{error_code, run};
use serde_json::json;

const STANDARD: &str = r#"{"Command":"CreateQueue","QueueName":"q"}"#;
const FIFO: &str = r#"{"Command":"CreateQueue","QueueName":"f.fifo","Attributes":{"Fifo":true}}"#;
const FIFO_CBD: &str = r#"{"Command":"CreateQueue","QueueName":"c.fifo","Attributes":{"Fifo":true,"ContentBasedDeduplication":true}}"#;

#[test]
fn message_ids_count_accepted_messages_over_all_queues() {
    let responses = run(&[
        STANDARD,
        r#"{"Command":"CreateQueue","QueueName":"p"}"#,
        r#"{"Command":"SendMessage","QueueName":"q","Body":"a"}"#,
        r#"{"Command":"SendMessage","QueueName":"q","Body":""}"#,
        r#"{"Command":"SendMessage","QueueName":"p","Body":"b"}"#,
    ]);
    assert_eq!(responses[2], json!({"ok": true, "MessageId": "m-1"}));
    assert_eq!(error_code(&responses[3]), "InvalidParameterValue");
    assert_eq!(responses[4], json!({"ok": true, "MessageId": "m-2"}));
}

#[test]
fn send_validates_parameters_in_order() {
    let long_body = "x".repeat(1025);
    let long_bytes = "\u{e9}".repeat(513);
    let big_ok = "\u{e9}".repeat(512);
    let too_long_body =
        format!(r#"{{"Command":"SendMessage","QueueName":"q","Body":"{long_body}"}}"#);
    let too_long_bytes =
        format!(r#"{{"Command":"SendMessage","QueueName":"q","Body":"{long_bytes}"}}"#);
    let max_bytes = format!(r#"{{"Command":"SendMessage","QueueName":"q","Body":"{big_ok}"}}"#);
    let cases = [
        (r#"{"Command":"SendMessage","Body":5}"#, "MissingParameter"),
        (
            r#"{"Command":"SendMessage","QueueName":"q"}"#,
            "MissingParameter",
        ),
        (
            r#"{"Command":"SendMessage","QueueName":"nope","Body":5}"#,
            "InvalidParameterValue",
        ),
        (
            r#"{"Command":"SendMessage","QueueName":"nope","Body":"a"}"#,
            "QueueDoesNotExist",
        ),
        (
            r#"{"Command":"SendMessage","QueueName":"q","Body":"a","DelaySeconds":901}"#,
            "InvalidParameterValue",
        ),
        (
            r#"{"Command":"SendMessage","QueueName":"q","Body":"a","GroupId":"bad id"}"#,
            "InvalidParameterValue",
        ),
        (
            r#"{"Command":"SendMessage","QueueName":"q","Body":"a","DeduplicationId":""}"#,
            "InvalidParameterValue",
        ),
        (
            r#"{"Command":"SendMessage","QueueName":"q","Body":"a","GroupId":"g"}"#,
            "UnsupportedOperation",
        ),
        (
            r#"{"Command":"SendMessage","QueueName":"q","Body":"a","DeduplicationId":"d"}"#,
            "UnsupportedOperation",
        ),
        (too_long_body.as_str(), "InvalidParameterValue"),
        (too_long_bytes.as_str(), "InvalidParameterValue"),
    ];
    for (line, code) in cases {
        let responses = run(&[STANDARD, line]);
        assert_eq!(error_code(&responses[1]), code, "{line}");
    }
    assert_eq!(
        run(&[STANDARD, &max_bytes])[1],
        json!({"ok": true, "MessageId": "m-1"})
    );
}

#[test]
fn fifo_send_rules_and_sequence_numbers() {
    let cases = [
        (
            r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"a","DelaySeconds":0,"GroupId":"g","DeduplicationId":"d"}"#,
            "UnsupportedOperation",
        ),
        (
            r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"a","DeduplicationId":"d"}"#,
            "MissingParameter",
        ),
        (
            r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"a","GroupId":"g"}"#,
            "MissingParameter",
        ),
    ];
    for (line, code) in cases {
        assert_eq!(error_code(&run(&[FIFO, line])[1]), code, "{line}");
    }
    let responses = run(&[
        FIFO,
        r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"a","GroupId":"g","DeduplicationId":"1"}"#,
        r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"a","GroupId":"h","DeduplicationId":"2"}"#,
    ]);
    assert_eq!(
        responses[1],
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
    assert_eq!(
        responses[2],
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2})
    );
}

#[test]
fn duplicate_sends_return_the_original_and_use_no_number() {
    let responses = run(&[
        FIFO_CBD,
        r#"{"Command":"SendMessage","QueueName":"c.fifo","Body":"a","GroupId":"g"}"#,
        r#"{"Command":"Tick","Seconds":299}"#,
        r#"{"Command":"SendMessage","QueueName":"c.fifo","Body":"a","GroupId":"other"}"#,
        r#"{"Command":"SendMessage","QueueName":"c.fifo","Body":"b","GroupId":"g"}"#,
        r#"{"Command":"Tick","Seconds":1}"#,
        r#"{"Command":"SendMessage","QueueName":"c.fifo","Body":"a","GroupId":"g"}"#,
        r#"{"Command":"SendMessage","QueueName":"c.fifo","Body":"a","GroupId":"g"}"#,
        r#"{"Command":"SendMessage","QueueName":"c.fifo","Body":"c","GroupId":"g","DeduplicationId":"a"}"#,
    ]);
    assert_eq!(
        responses[1],
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
    assert_eq!(responses[3], responses[1]);
    assert_eq!(
        responses[4],
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2})
    );
    assert_eq!(
        responses[6],
        json!({"ok": true, "MessageId": "m-3", "SequenceNumber": 3})
    );
    assert_eq!(responses[7], responses[6]);
    assert_eq!(responses[8], responses[6]);
}

#[test]
fn deduplication_survives_purge_and_is_per_queue() {
    let responses = run(&[
        FIFO_CBD,
        FIFO,
        r#"{"Command":"SendMessage","QueueName":"c.fifo","Body":"a","GroupId":"g"}"#,
        r#"{"Command":"PurgeQueue","QueueName":"c.fifo"}"#,
        r#"{"Command":"SendMessage","QueueName":"c.fifo","Body":"a","GroupId":"g"}"#,
        r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"a","GroupId":"g","DeduplicationId":"a"}"#,
        r#"{"Command":"SendMessage","QueueName":"c.fifo","Body":"z","GroupId":"g"}"#,
    ]);
    assert_eq!(responses[4], responses[2]);
    assert_eq!(
        responses[5],
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 1})
    );
    assert_eq!(
        responses[6],
        json!({"ok": true, "MessageId": "m-3", "SequenceNumber": 2})
    );
}

#[test]
fn delay_comes_from_the_request_or_the_queue_at_send_time() {
    let responses = run(&[
        r#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"DelaySeconds":10}}"#,
        r#"{"Command":"SendMessage","QueueName":"q","Body":"a"}"#,
        r#"{"Command":"SendMessage","QueueName":"q","Body":"b","DelaySeconds":0}"#,
        r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"DelaySeconds":0}}"#,
        r#"{"Command":"GetQueueAttributes","QueueName":"q"}"#,
        r#"{"Command":"Tick","Seconds":10}"#,
        r#"{"Command":"GetQueueAttributes","QueueName":"q"}"#,
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
fn retention_removes_messages_at_once_and_never_returns_them() {
    let responses = run(&[
        r#"{"Command":"CreateQueue","QueueName":"q"}"#,
        r#"{"Command":"SendMessage","QueueName":"q","Body":"a"}"#,
        r#"{"Command":"Tick","Seconds":59}"#,
        r#"{"Command":"SendMessage","QueueName":"q","Body":"b"}"#,
        r#"{"Command":"Tick","Seconds":1}"#,
        r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"RetentionSeconds":60}}"#,
        r#"{"Command":"GetQueueAttributes","QueueName":"q"}"#,
        r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"RetentionSeconds":1000}}"#,
        r#"{"Command":"GetQueueAttributes","QueueName":"q"}"#,
        r#"{"Command":"Tick","Seconds":59}"#,
        r#"{"Command":"GetQueueAttributes","QueueName":"q"}"#,
        r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"RetentionSeconds":60}}"#,
        r#"{"Command":"GetQueueAttributes","QueueName":"q"}"#,
    ]);
    assert_eq!(responses[6]["Counts"]["Visible"], 1);
    assert_eq!(responses[8]["Counts"]["Visible"], 1);
    assert_eq!(responses[10]["Counts"]["Visible"], 1);
    assert_eq!(responses[12]["Counts"]["Visible"], 0);
}

#[test]
fn a_queue_holds_at_most_one_hundred_thousand_messages() {
    let mut commands = vec![STANDARD.to_owned()];
    commands.extend(
        (0..100_001).map(|_| r#"{"Command":"SendMessage","QueueName":"q","Body":"a"}"#.to_owned()),
    );
    commands.push(r#"{"Command":"PurgeQueue","QueueName":"q"}"#.to_owned());
    commands.push(r#"{"Command":"SendMessage","QueueName":"q","Body":"a"}"#.to_owned());
    let refs: Vec<&str> = commands.iter().map(String::as_str).collect();
    let responses = run(&refs);
    assert_eq!(
        responses[100_000],
        json!({"ok": true, "MessageId": "m-100000"})
    );
    assert_eq!(error_code(&responses[100_001]), "LimitExceeded");
    assert_eq!(
        responses[100_003],
        json!({"ok": true, "MessageId": "m-100001"})
    );
}
