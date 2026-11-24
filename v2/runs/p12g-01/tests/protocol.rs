//! End-to-end sessions: command lines in, reply lines out.

use std::io::Cursor;

use serde_json::{Value, json};

/// Runs the lines through miniq and returns one parsed reply per line.
fn session(lines: &[&str]) -> Vec<Value> {
    let input = lines.join("\n") + "\n";
    run_bytes(input.into_bytes())
}

fn run_bytes(input: Vec<u8>) -> Vec<Value> {
    let mut output = Vec::new();
    miniq::run(Cursor::new(input), &mut output).unwrap();
    let text = String::from_utf8(output).unwrap();
    assert!(
        text.is_empty() || text.ends_with('\n'),
        "replies end with a newline"
    );
    text.lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn ok() -> Value {
    json!({"ok": true})
}

fn error_code(reply: &Value) -> &str {
    assert_eq!(reply["ok"], json!(false), "reply {reply}");
    reply["error"]["code"].as_str().unwrap()
}

/// Runs the lines and compares each reply with its expected error code,
/// where "ok" stands for any success.
fn assert_codes(lines: &[&str], expected: &[&str]) {
    let replies = session(lines);
    let codes: Vec<&str> = replies
        .iter()
        .map(|reply| {
            if reply["ok"] == json!(true) {
                "ok"
            } else {
                error_code(reply)
            }
        })
        .collect();
    assert_eq!(codes, expected);
}

#[test]
fn every_line_gets_one_reply_in_order_and_failures_do_not_stop_the_session() {
    let replies = session(&["", "garbage", r#"{"Command":"Tick","Seconds":5}"#, "[1]"]);
    assert_eq!(replies.len(), 4);
    assert_eq!(error_code(&replies[0]), "InvalidRequest");
    assert_eq!(error_code(&replies[1]), "InvalidRequest");
    assert_eq!(replies[2], json!({"ok": true, "Now": 5}));
    assert_eq!(error_code(&replies[3]), "InvalidRequest");
}

#[test]
fn final_line_without_newline_is_a_command() {
    let replies = run_bytes(br#"{"Command":"Tick","Seconds":2}"#.to_vec());
    assert_eq!(replies, [json!({"ok": true, "Now": 2})]);
}

#[test]
fn no_input_gives_no_output() {
    assert_eq!(run_bytes(Vec::new()), Vec::<Value>::new());
}

#[test]
fn invalid_utf8_line_is_an_invalid_request_and_the_next_line_still_runs() {
    let replies = run_bytes(b"\xff\xfe\n{\"Command\":\"Tick\",\"Seconds\":1}\n".to_vec());
    assert_eq!(replies.len(), 2);
    assert_eq!(error_code(&replies[0]), "InvalidRequest");
    assert_eq!(replies[1], json!({"ok": true, "Now": 1}));
}

#[test]
fn create_queue_is_idempotent_for_equal_attributes_only() {
    assert_codes(
        &[
            r#"{"Command":"CreateQueue","QueueName":"q"}"#,
            r#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"VisibilityTimeout":30,"DelaySeconds":null}}"#,
            r#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"VisibilityTimeout":31}}"#,
        ],
        &["ok", "ok", "QueueAlreadyExists"],
    );
}

#[test]
fn create_queue_error_precedence() {
    assert_codes(
        &[
            r#"{"Command":"CreateQueue","QueueName":"a.fifo"}"#,
            r#"{"Command":"CreateQueue","QueueName":"a","Attributes":{"Fifo":true}}"#,
            r#"{"Command":"CreateQueue","QueueName":"a","Attributes":{"ContentBasedDeduplication":true}}"#,
            r#"{"Command":"CreateQueue","QueueName":"a","Attributes":{"MaxReceiveCount":3}}"#,
            r#"{"Command":"CreateQueue","QueueName":"a","Attributes":{"DeadLetterQueue":"d","MaxReceiveCount":3}}"#,
            r#"{"Command":"CreateQueue","QueueName":"a b"}"#,
            r#"{"Command":"CreateQueue"}"#,
        ],
        &[
            "QueueNameInvalid",
            "QueueNameInvalid",
            "InvalidParameterValue",
            "InvalidParameterValue",
            "QueueDoesNotExist",
            "QueueNameInvalid",
            "MissingParameter",
        ],
    );
}

#[test]
fn dead_letter_queue_must_share_the_type() {
    assert_codes(
        &[
            r#"{"Command":"CreateQueue","QueueName":"d.fifo","Attributes":{"Fifo":true}}"#,
            r#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"DeadLetterQueue":"d.fifo","MaxReceiveCount":2}}"#,
        ],
        &["ok", "InvalidParameterValue"],
    );
}

#[test]
fn queue_limit_is_1000() {
    let creates: Vec<String> = (0..1001)
        .map(|n| format!(r#"{{"Command":"CreateQueue","QueueName":"q{n}"}}"#))
        .collect();
    let lines: Vec<&str> = creates.iter().map(String::as_str).collect();
    let replies = session(&lines);
    assert!(replies[..1000].iter().all(|r| *r == ok()));
    assert_eq!(error_code(&replies[1000]), "LimitExceeded");
}

#[test]
fn get_queue_attributes_reports_defaults_and_counts() {
    let replies = session(&[
        r#"{"Command":"CreateQueue","QueueName":"q"}"#,
        r#"{"Command":"SendMessage","QueueName":"q","Body":"a"}"#,
        r#"{"Command":"SendMessage","QueueName":"q","Body":"b","DelaySeconds":10}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"q"}"#,
        r#"{"Command":"GetQueueAttributes","QueueName":"q"}"#,
        r#"{"Command":"GetQueueAttributes","QueueName":"Q"}"#,
    ]);
    assert_eq!(
        replies[4],
        json!({
            "ok": true,
            "Attributes": {
                "VisibilityTimeout": 30, "DelaySeconds": 0, "RetentionSeconds": 345600,
                "Fifo": false, "ContentBasedDeduplication": false
            },
            "Counts": {"Visible": 0, "InFlight": 1, "Delayed": 1}
        })
    );
    assert_eq!(error_code(&replies[5]), "QueueDoesNotExist");
}

#[test]
fn list_queues_sorts_by_byte_value_and_filters_by_prefix() {
    let replies = session(&[
        r#"{"Command":"CreateQueue","QueueName":"b"}"#,
        r#"{"Command":"CreateQueue","QueueName":"_x"}"#,
        r#"{"Command":"CreateQueue","QueueName":"B"}"#,
        r#"{"Command":"CreateQueue","QueueName":"a.fifo","Attributes":{"Fifo":true}}"#,
        r#"{"Command":"CreateQueue","QueueName":"a-1"}"#,
        r#"{"Command":"CreateQueue","QueueName":"a1"}"#,
        r#"{"Command":"ListQueues"}"#,
        r#"{"Command":"ListQueues","Prefix":"a"}"#,
        r#"{"Command":"ListQueues","Prefix":"zz"}"#,
        r#"{"Command":"ListQueues","Prefix":7}"#,
    ]);
    assert_eq!(
        replies[6],
        json!({"ok": true, "QueueNames": ["B", "_x", "a-1", "a.fifo", "a1", "b"]})
    );
    assert_eq!(
        replies[7],
        json!({"ok": true, "QueueNames": ["a-1", "a.fifo", "a1"]})
    );
    assert_eq!(replies[8], json!({"ok": true, "QueueNames": []}));
    assert_eq!(error_code(&replies[9]), "InvalidParameterValue");
}

#[test]
fn standard_queue_send_receive_delete() {
    let replies = session(&[
        r#"{"Command":"CreateQueue","QueueName":"q"}"#,
        r#"{"Command":"SendMessage","QueueName":"q","Body":"one"}"#,
        r#"{"Command":"Tick","Seconds":4}"#,
        r#"{"Command":"SendMessage","QueueName":"q","Body":"two"}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"q","MaxMessages":5}"#,
        r#"{"Command":"DeleteMessage","QueueName":"q","ReceiptHandle":"r-1"}"#,
        r#"{"Command":"DeleteMessage","QueueName":"q","ReceiptHandle":"r-1"}"#,
        r#"{"Command":"DeleteMessage","QueueName":"q","ReceiptHandle":"r-3"}"#,
        r#"{"Command":"DeleteMessage","QueueName":"q","ReceiptHandle":"r-01"}"#,
    ]);
    assert_eq!(replies[1], json!({"ok": true, "MessageId": "m-1"}));
    assert_eq!(replies[3], json!({"ok": true, "MessageId": "m-2"}));
    assert_eq!(
        replies[4],
        json!({"ok": true, "Messages": [
            {"MessageId": "m-1", "ReceiptHandle": "r-1", "Body": "one", "ReceiveCount": 1,
             "SentAt": 0, "FirstReceivedAt": 4},
            {"MessageId": "m-2", "ReceiptHandle": "r-2", "Body": "two", "ReceiveCount": 1,
             "SentAt": 4, "FirstReceivedAt": 4}
        ]})
    );
    assert_eq!(replies[5], ok());
    assert_eq!(error_code(&replies[6]), "ReceiptHandleExpired");
    assert_eq!(error_code(&replies[7]), "ReceiptHandleInvalid");
    assert_eq!(error_code(&replies[8]), "ReceiptHandleInvalid");
}

#[test]
fn handle_from_another_queue_is_expired_and_checked_after_queue_lookup() {
    assert_codes(
        &[
            r#"{"Command":"CreateQueue","QueueName":"a"}"#,
            r#"{"Command":"CreateQueue","QueueName":"b"}"#,
            r#"{"Command":"SendMessage","QueueName":"a","Body":"x"}"#,
            r#"{"Command":"ReceiveMessage","QueueName":"a"}"#,
            r#"{"Command":"DeleteMessage","QueueName":"b","ReceiptHandle":"r-1"}"#,
            r#"{"Command":"DeleteMessage","QueueName":"c","ReceiptHandle":"nope"}"#,
        ],
        &[
            "ok",
            "ok",
            "ok",
            "ok",
            "ReceiptHandleExpired",
            "QueueDoesNotExist",
        ],
    );
}

#[test]
fn visibility_timeout_ends_exactly_at_the_deadline_and_redelivery_expires_the_old_handle() {
    let replies = session(&[
        r#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"VisibilityTimeout":10}}"#,
        r#"{"Command":"SendMessage","QueueName":"q","Body":"x"}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"q"}"#,
        r#"{"Command":"Tick","Seconds":9}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"q"}"#,
        r#"{"Command":"Tick","Seconds":1}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"q"}"#,
        r#"{"Command":"DeleteMessage","QueueName":"q","ReceiptHandle":"r-1"}"#,
        r#"{"Command":"DeleteMessage","QueueName":"q","ReceiptHandle":"r-2"}"#,
    ]);
    assert_eq!(replies[4], json!({"ok": true, "Messages": []}));
    assert_eq!(replies[6]["Messages"][0]["ReceiveCount"], json!(2));
    assert_eq!(replies[6]["Messages"][0]["FirstReceivedAt"], json!(0));
    assert_eq!(error_code(&replies[7]), "ReceiptHandleExpired");
    assert_eq!(replies[8], ok());
}

#[test]
fn change_message_visibility_replaces_the_deadline() {
    let replies = session(&[
        r#"{"Command":"CreateQueue","QueueName":"q"}"#,
        r#"{"Command":"SendMessage","QueueName":"q","Body":"x"}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"q","VisibilityTimeout":5}"#,
        r#"{"Command":"Tick","Seconds":4}"#,
        r#"{"Command":"ChangeMessageVisibility","QueueName":"q","ReceiptHandle":"r-1","VisibilityTimeout":10}"#,
        r#"{"Command":"Tick","Seconds":9}"#,
        r#"{"Command":"GetQueueAttributes","QueueName":"q"}"#,
        r#"{"Command":"Tick","Seconds":1}"#,
        r#"{"Command":"GetQueueAttributes","QueueName":"q"}"#,
        r#"{"Command":"ChangeMessageVisibility","QueueName":"q","ReceiptHandle":"r-1","VisibilityTimeout":10}"#,
        r#"{"Command":"ChangeMessageVisibility","QueueName":"q","ReceiptHandle":"r-1"}"#,
    ]);
    assert_eq!(replies[4], ok());
    assert_eq!(
        replies[6]["Counts"],
        json!({"Visible": 0, "InFlight": 1, "Delayed": 0})
    );
    assert_eq!(
        replies[8]["Counts"],
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(error_code(&replies[9]), "ReceiptHandleExpired");
    assert_eq!(error_code(&replies[10]), "MissingParameter");
}

#[test]
fn zero_visibility_timeout_leaves_the_handle_unusable() {
    assert_codes(
        &[
            r#"{"Command":"CreateQueue","QueueName":"q"}"#,
            r#"{"Command":"SendMessage","QueueName":"q","Body":"x"}"#,
            r#"{"Command":"ReceiveMessage","QueueName":"q","VisibilityTimeout":0}"#,
            r#"{"Command":"DeleteMessage","QueueName":"q","ReceiptHandle":"r-1"}"#,
        ],
        &["ok", "ok", "ok", "ReceiptHandleExpired"],
    );
}

#[test]
fn retention_removes_messages_at_once_and_is_not_undone() {
    let replies = session(&[
        r#"{"Command":"CreateQueue","QueueName":"q"}"#,
        r#"{"Command":"SendMessage","QueueName":"q","Body":"x"}"#,
        r#"{"Command":"Tick","Seconds":100}"#,
        r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"RetentionSeconds":100}}"#,
        r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"RetentionSeconds":1000}}"#,
        r#"{"Command":"GetQueueAttributes","QueueName":"q"}"#,
    ]);
    assert_eq!(
        replies[5]["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn set_queue_attributes_rules_in_order() {
    assert_codes(
        &[
            r#"{"Command":"CreateQueue","QueueName":"q"}"#,
            r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"Fifo":false}}"#,
            r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"ContentBasedDeduplication":true}}"#,
            r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"MaxReceiveCount":2}}"#,
            r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"DeadLetterQueue":"nope","MaxReceiveCount":2}}"#,
            r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"DeadLetterQueue":"q","MaxReceiveCount":2}}"#,
            r#"{"Command":"SetQueueAttributes","QueueName":"nope","Attributes":{}}"#,
            r#"{"Command":"SetQueueAttributes","QueueName":"q"}"#,
            r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{}}"#,
        ],
        &[
            "ok",
            "UnsupportedOperation",
            "InvalidParameterValue",
            "InvalidParameterValue",
            "QueueDoesNotExist",
            "InvalidParameterValue",
            "QueueDoesNotExist",
            "MissingParameter",
            "ok",
        ],
    );
}

#[test]
fn failed_set_queue_attributes_changes_nothing() {
    let replies = session(&[
        r#"{"Command":"CreateQueue","QueueName":"q"}"#,
        r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"VisibilityTimeout":5,"DeadLetterQueue":"nope","MaxReceiveCount":2}}"#,
        r#"{"Command":"GetQueueAttributes","QueueName":"q"}"#,
    ]);
    assert_eq!(replies[2]["Attributes"]["VisibilityTimeout"], json!(30));
}

#[test]
fn dead_letter_flow_and_delete_queue_clears_the_configuration() {
    let replies = session(&[
        r#"{"Command":"CreateQueue","QueueName":"dlq","Attributes":{"RetentionSeconds":60}}"#,
        r#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"VisibilityTimeout":0,"DeadLetterQueue":"dlq","MaxReceiveCount":2}}"#,
        r#"{"Command":"SendMessage","QueueName":"q","Body":"x"}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"q"}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"q"}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"q"}"#,
        r#"{"Command":"GetQueueAttributes","QueueName":"dlq"}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"dlq"}"#,
        r#"{"Command":"DeleteQueue","QueueName":"dlq"}"#,
        r#"{"Command":"GetQueueAttributes","QueueName":"q"}"#,
        r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"MaxReceiveCount":2}}"#,
    ]);
    assert_eq!(replies[3]["Messages"][0]["ReceiveCount"], json!(1));
    assert_eq!(replies[4]["Messages"][0]["ReceiveCount"], json!(2));
    assert_eq!(replies[5], json!({"ok": true, "Messages": []}));
    assert_eq!(
        replies[6]["Counts"],
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(
        replies[7]["Messages"][0],
        json!({"MessageId": "m-1", "ReceiptHandle": "r-3", "Body": "x", "ReceiveCount": 1,
               "SentAt": 0, "FirstReceivedAt": 0})
    );
    assert_eq!(
        replies[9]["Attributes"],
        json!({"VisibilityTimeout": 0, "DelaySeconds": 0, "RetentionSeconds": 345600,
               "Fifo": false, "ContentBasedDeduplication": false})
    );
    assert_eq!(error_code(&replies[10]), "InvalidParameterValue");
}

#[test]
fn dead_lettered_message_that_is_too_old_for_the_dead_letter_queue_disappears() {
    let replies = session(&[
        r#"{"Command":"CreateQueue","QueueName":"dlq","Attributes":{"RetentionSeconds":60}}"#,
        r#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"VisibilityTimeout":0,"DeadLetterQueue":"dlq","MaxReceiveCount":1}}"#,
        r#"{"Command":"SendMessage","QueueName":"q","Body":"x"}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"q"}"#,
        r#"{"Command":"Tick","Seconds":60}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"q"}"#,
        r#"{"Command":"GetQueueAttributes","QueueName":"dlq"}"#,
    ]);
    assert_eq!(replies[5], json!({"ok": true, "Messages": []}));
    assert_eq!(
        replies[6]["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn purge_removes_everything_but_fifo_sequence_numbers_and_dedup_carry_on() {
    let replies = session(&[
        r#"{"Command":"CreateQueue","QueueName":"q.fifo","Attributes":{"Fifo":true}}"#,
        r#"{"Command":"SendMessage","QueueName":"q.fifo","Body":"a","GroupId":"g","DeduplicationId":"d1"}"#,
        r#"{"Command":"PurgeQueue","QueueName":"q.fifo"}"#,
        r#"{"Command":"SendMessage","QueueName":"q.fifo","Body":"a","GroupId":"g","DeduplicationId":"d1"}"#,
        r#"{"Command":"SendMessage","QueueName":"q.fifo","Body":"b","GroupId":"g","DeduplicationId":"d2"}"#,
        r#"{"Command":"PurgeQueue","QueueName":"nope"}"#,
    ]);
    assert_eq!(
        replies[3],
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
    assert_eq!(
        replies[4],
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2})
    );
    assert_eq!(error_code(&replies[5]), "QueueDoesNotExist");
}

#[test]
fn send_message_error_precedence() {
    assert_codes(
        &[
            r#"{"Command":"CreateQueue","QueueName":"s"}"#,
            r#"{"Command":"CreateQueue","QueueName":"f.fifo","Attributes":{"Fifo":true}}"#,
            r#"{"Command":"SendMessage","QueueName":"s","Body":"x","GroupId":"g"}"#,
            r#"{"Command":"SendMessage","QueueName":"s","Body":"x","DeduplicationId":"d"}"#,
            r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"x","DelaySeconds":1}"#,
            r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"x","DeduplicationId":"d"}"#,
            r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"x","GroupId":"g"}"#,
            r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"x","GroupId":"g.1","DeduplicationId":"d"}"#,
            r#"{"Command":"SendMessage","QueueName":"s","Body":""}"#,
            r#"{"Command":"SendMessage","QueueName":"s","Body":"x","DelaySeconds":901}"#,
            r#"{"Command":"SendMessage","QueueName":"nope","Body":"x"}"#,
        ],
        &[
            "ok",
            "ok",
            "UnsupportedOperation",
            "UnsupportedOperation",
            "UnsupportedOperation",
            "MissingParameter",
            "MissingParameter",
            "InvalidParameterValue",
            "InvalidParameterValue",
            "InvalidParameterValue",
            "QueueDoesNotExist",
        ],
    );
}

#[test]
fn failed_send_does_not_use_up_a_message_id() {
    let replies = session(&[
        r#"{"Command":"CreateQueue","QueueName":"q"}"#,
        r#"{"Command":"SendMessage","QueueName":"q","Body":"x","GroupId":"g"}"#,
        r#"{"Command":"SendMessage","QueueName":"q","Body":"x"}"#,
    ]);
    assert_eq!(replies[2], json!({"ok": true, "MessageId": "m-1"}));
}

#[test]
fn delay_comes_from_the_request_or_the_queue_at_send_time() {
    let replies = session(&[
        r#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"DelaySeconds":10}}"#,
        r#"{"Command":"SendMessage","QueueName":"q","Body":"a"}"#,
        r#"{"Command":"SendMessage","QueueName":"q","Body":"b","DelaySeconds":0}"#,
        r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"DelaySeconds":20}}"#,
        r#"{"Command":"Tick","Seconds":10}"#,
        r#"{"Command":"GetQueueAttributes","QueueName":"q"}"#,
    ]);
    assert_eq!(
        replies[5]["Counts"],
        json!({"Visible": 2, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn fifo_deduplication_window_and_content_based_ids() {
    let replies = session(&[
        r#"{"Command":"CreateQueue","QueueName":"q.fifo","Attributes":{"Fifo":true,"ContentBasedDeduplication":true}}"#,
        r#"{"Command":"SendMessage","QueueName":"q.fifo","Body":"same","GroupId":"g"}"#,
        r#"{"Command":"Tick","Seconds":299}"#,
        r#"{"Command":"SendMessage","QueueName":"q.fifo","Body":"same","GroupId":"other"}"#,
        r#"{"Command":"Tick","Seconds":1}"#,
        r#"{"Command":"SendMessage","QueueName":"q.fifo","Body":"same","GroupId":"g"}"#,
        r#"{"Command":"SendMessage","QueueName":"q.fifo","Body":"x","GroupId":"g","DeduplicationId":"same"}"#,
    ]);
    let first = json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1});
    let second = json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2});
    assert_eq!(replies[1], first);
    assert_eq!(replies[3], first);
    assert_eq!(replies[5], second);
    assert_eq!(replies[6], second);
}

#[test]
fn fifo_receive_returns_one_message_per_group_in_arrival_order() {
    let replies = session(&[
        r#"{"Command":"CreateQueue","QueueName":"q.fifo","Attributes":{"Fifo":true}}"#,
        r#"{"Command":"SendMessage","QueueName":"q.fifo","Body":"a1","GroupId":"a","DeduplicationId":"1"}"#,
        r#"{"Command":"SendMessage","QueueName":"q.fifo","Body":"a2","GroupId":"a","DeduplicationId":"2"}"#,
        r#"{"Command":"SendMessage","QueueName":"q.fifo","Body":"b1","GroupId":"b","DeduplicationId":"3"}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"q.fifo","MaxMessages":10}"#,
    ]);
    assert_eq!(
        replies[4],
        json!({"ok": true, "Messages": [
            {"MessageId": "m-1", "ReceiptHandle": "r-1", "Body": "a1", "ReceiveCount": 1,
             "SentAt": 0, "FirstReceivedAt": 0, "GroupId": "a", "SequenceNumber": 1},
            {"MessageId": "m-3", "ReceiptHandle": "r-2", "Body": "b1", "ReceiveCount": 1,
             "SentAt": 0, "FirstReceivedAt": 0, "GroupId": "b", "SequenceNumber": 3}
        ]})
    );
}

#[test]
fn tick_bounds() {
    assert_codes(
        &[
            r#"{"Command":"Tick"}"#,
            r#"{"Command":"Tick","Seconds":1209601}"#,
            r#"{"Command":"Tick","Seconds":1.0}"#,
        ],
        &[
            "MissingParameter",
            "InvalidParameterValue",
            "InvalidParameterValue",
        ],
    );
}

#[test]
fn queue_holds_at_most_100000_messages() {
    let mut lines = vec![r#"{"Command":"CreateQueue","QueueName":"q"}"#.to_owned()];
    lines.extend(
        (0..100_001)
            .map(|n| format!(r#"{{"Command":"SendMessage","QueueName":"q","Body":"m{n}"}}"#)),
    );
    let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
    let replies = session(&refs);
    assert_eq!(replies[100_000]["MessageId"], json!("m-100000"));
    assert_eq!(error_code(&replies[100_001]), "LimitExceeded");
}
