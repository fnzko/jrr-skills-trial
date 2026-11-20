mod common;

use common::{error_code, run};
use serde_json::{Value, json};

const CREATE_Q: &str = r#"{"Command": "CreateQueue", "QueueName": "q"}"#;

fn send(body: &str) -> String {
    json!({"Command": "SendMessage", "QueueName": "q", "Body": body}).to_string()
}

fn receive() -> &'static str {
    r#"{"Command": "ReceiveMessage", "QueueName": "q"}"#
}

fn tick(seconds: u32) -> String {
    json!({"Command": "Tick", "Seconds": seconds}).to_string()
}

fn counts(response: &Value) -> Value {
    response["Counts"].clone()
}

#[test]
fn ids_and_handles_count_over_all_queues() {
    let out = run(&[
        CREATE_Q,
        r#"{"Command": "CreateQueue", "QueueName": "other"}"#,
        &send("a"),
        r#"{"Command": "SendMessage", "QueueName": "other", "Body": "b"}"#,
        r#"{"Command": "SendMessage", "QueueName": "nope", "Body": "x"}"#,
        &send("c"),
        r#"{"Command": "ReceiveMessage", "QueueName": "other"}"#,
        r#"{"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 10}"#,
    ]);
    assert_eq!(out[2], json!({"ok": true, "MessageId": "m-1"}));
    assert_eq!(out[3], json!({"ok": true, "MessageId": "m-2"}));
    assert_eq!(error_code(&out[4]), "QueueDoesNotExist");
    assert_eq!(out[5], json!({"ok": true, "MessageId": "m-3"}));
    assert_eq!(
        out[6]["Messages"],
        json!([{
            "MessageId": "m-2", "ReceiptHandle": "r-1", "Body": "b",
            "ReceiveCount": 1, "SentAt": 0, "FirstReceivedAt": 0
        }])
    );
    assert_eq!(
        out[7]["Messages"],
        json!([
            {"MessageId": "m-1", "ReceiptHandle": "r-2", "Body": "a", "ReceiveCount": 1, "SentAt": 0, "FirstReceivedAt": 0},
            {"MessageId": "m-3", "ReceiptHandle": "r-3", "Body": "c", "ReceiveCount": 1, "SentAt": 0, "FirstReceivedAt": 0}
        ])
    );
}

#[test]
fn send_checks_its_parameters() {
    let kilobyte = "x".repeat(1024);
    let too_long = "x".repeat(1025);
    // 512 two-byte characters fit; 513 do not.
    let wide_ok = "é".repeat(512);
    let wide_bad = "é".repeat(513);
    let out = run(&[
        CREATE_Q,
        r#"{"Command": "SendMessage", "QueueName": "q"}"#,
        r#"{"Command": "SendMessage", "Body": "x"}"#,
        &send(""),
        &send(&kilobyte),
        &send(&too_long),
        &send(&wide_ok),
        &send(&wide_bad),
        r#"{"Command": "SendMessage", "QueueName": "q", "Body": 1}"#,
        r#"{"Command": "SendMessage", "QueueName": "q", "Body": "x", "DelaySeconds": 901}"#,
        r#"{"Command": "SendMessage", "QueueName": "q", "Body": "x", "GroupId": ""}"#,
        r#"{"Command": "SendMessage", "QueueName": "q", "Body": "x", "GroupId": "a b"}"#,
        r#"{"Command": "SendMessage", "QueueName": "q", "Body": "x", "DeduplicationId": 5}"#,
        r#"{"Command": "SendMessage", "QueueName": "q", "Body": "x", "DelaySeconds": 900}"#,
    ]);
    let codes: Vec<&str> = out[1..]
        .iter()
        .map(|r| if r["ok"] == true { "ok" } else { error_code(r) })
        .collect();
    assert_eq!(
        codes,
        [
            "MissingParameter",
            "MissingParameter",
            "InvalidParameterValue",
            "ok",
            "InvalidParameterValue",
            "ok",
            "InvalidParameterValue",
            "InvalidParameterValue",
            "InvalidParameterValue",
            "InvalidParameterValue",
            "InvalidParameterValue",
            "InvalidParameterValue",
            "ok",
        ]
    );
}

#[test]
fn failed_sends_do_not_use_up_message_ids() {
    let out = run(&[CREATE_Q, &send(""), &send("a")]);
    assert_eq!(out[2], json!({"ok": true, "MessageId": "m-1"}));
}

#[test]
fn standard_queue_rejects_fifo_parameters() {
    let out = run(&[
        CREATE_Q,
        r#"{"Command": "SendMessage", "QueueName": "q", "Body": "x", "GroupId": "g"}"#,
        r#"{"Command": "SendMessage", "QueueName": "q", "Body": "x", "DeduplicationId": "d"}"#,
    ]);
    assert_eq!(error_code(&out[1]), "UnsupportedOperation");
    assert_eq!(error_code(&out[2]), "UnsupportedOperation");
}

#[test]
fn delay_hides_a_message_until_its_time() {
    let out = run(&[
        r#"{"Command": "CreateQueue", "QueueName": "q", "Attributes": {"DelaySeconds": 10}}"#,
        &send("queue delay"),
        r#"{"Command": "SendMessage", "QueueName": "q", "Body": "own delay", "DelaySeconds": 3}"#,
        r#"{"Command": "SendMessage", "QueueName": "q", "Body": "no delay", "DelaySeconds": 0}"#,
        r#"{"Command": "GetQueueAttributes", "QueueName": "q"}"#,
        &tick(2),
        receive(),
        &tick(1),
        receive(),
        &tick(7),
        receive(),
    ]);
    assert_eq!(
        counts(&out[4]),
        json!({"Visible": 1, "InFlight": 0, "Delayed": 2})
    );
    assert_eq!(out[6]["Messages"][0]["Body"], "no delay");
    assert_eq!(out[8]["Messages"][0]["Body"], "own delay");
    assert_eq!(out[10]["Messages"][0]["Body"], "queue delay");
}

#[test]
fn a_changed_queue_delay_applies_to_later_sends_only() {
    let out = run(&[
        r#"{"Command": "CreateQueue", "QueueName": "q", "Attributes": {"DelaySeconds": 10}}"#,
        &send("first"),
        r#"{"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"DelaySeconds": 0}}"#,
        &send("second"),
        r#"{"Command": "GetQueueAttributes", "QueueName": "q"}"#,
    ]);
    assert_eq!(
        counts(&out[4]),
        json!({"Visible": 1, "InFlight": 0, "Delayed": 1})
    );
}

#[test]
fn receive_returns_the_oldest_first_and_hides_messages_for_the_visibility_timeout() {
    let out = run(&[
        CREATE_Q,
        &send("a"),
        &send("b"),
        r#"{"Command": "ReceiveMessage", "QueueName": "q", "VisibilityTimeout": 5}"#,
        r#"{"Command": "GetQueueAttributes", "QueueName": "q"}"#,
        &tick(4),
        receive(),
        &tick(1),
        receive(),
    ]);
    assert_eq!(out[3]["Messages"][0]["Body"], "a");
    assert_eq!(
        counts(&out[4]),
        json!({"Visible": 1, "InFlight": 1, "Delayed": 0})
    );
    // `b` was taken with the queue's 30 s timeout; `a` is visible again at t=5.
    assert_eq!(out[6]["Messages"][0]["Body"], "b");
    assert_eq!(out[8]["Messages"][0]["Body"], "a");
    assert_eq!(out[8]["Messages"][0]["ReceiveCount"], 2);
    assert_eq!(out[8]["Messages"][0]["FirstReceivedAt"], 0);
    assert_eq!(out[8]["Messages"][0]["ReceiptHandle"], "r-3");
}

#[test]
fn receive_checks_its_parameters() {
    let out = run(&[
        CREATE_Q,
        r#"{"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 0}"#,
        r#"{"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 11}"#,
        r#"{"Command": "ReceiveMessage", "QueueName": "q", "VisibilityTimeout": 43201}"#,
        r#"{"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 10, "VisibilityTimeout": 43200}"#,
        r#"{"Command": "ReceiveMessage"}"#,
    ]);
    assert_eq!(error_code(&out[1]), "InvalidParameterValue");
    assert_eq!(error_code(&out[2]), "InvalidParameterValue");
    assert_eq!(error_code(&out[3]), "InvalidParameterValue");
    assert_eq!(out[4], json!({"ok": true, "Messages": []}));
    assert_eq!(error_code(&out[5]), "MissingParameter");
}

#[test]
fn delete_message_removes_a_message_that_is_in_flight() {
    let out = run(&[
        CREATE_Q,
        &send("a"),
        receive(),
        r#"{"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": "r-1"}"#,
        r#"{"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": "r-1"}"#,
        r#"{"Command": "GetQueueAttributes", "QueueName": "q"}"#,
    ]);
    assert_eq!(out[3], json!({"ok": true}));
    assert_eq!(error_code(&out[4]), "ReceiptHandleExpired");
    assert_eq!(
        counts(&out[5]),
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn receipt_handles_are_compared_as_exact_strings() {
    let out = run(&[
        CREATE_Q,
        &send("a"),
        receive(),
        r#"{"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": "r-01"}"#,
        r#"{"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": "R-1"}"#,
        r#"{"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": "r-1 "}"#,
        r#"{"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": "r-+1"}"#,
        r#"{"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": "r-2"}"#,
        r#"{"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": "r-0"}"#,
        r#"{"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": ""}"#,
        r#"{"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": "r-99999999999999999999999"}"#,
        r#"{"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": 1}"#,
        r#"{"Command": "DeleteMessage", "QueueName": "q"}"#,
        r#"{"Command": "DeleteMessage", "QueueName": "nope", "ReceiptHandle": "bogus"}"#,
    ]);
    for response in &out[3..9] {
        assert_eq!(error_code(response), "ReceiptHandleInvalid", "{response}");
    }
    assert_eq!(error_code(&out[9]), "ReceiptHandleInvalid");
    assert_eq!(error_code(&out[10]), "ReceiptHandleInvalid");
    assert_eq!(error_code(&out[11]), "InvalidParameterValue");
    assert_eq!(error_code(&out[12]), "MissingParameter");
    assert_eq!(error_code(&out[13]), "QueueDoesNotExist");
}

#[test]
fn a_handle_expires_when_the_message_is_delivered_again_or_the_timeout_ends() {
    let out = run(&[
        CREATE_Q,
        &send("a"),
        r#"{"Command": "ReceiveMessage", "QueueName": "q", "VisibilityTimeout": 10}"#,
        &tick(10),
        r#"{"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": "r-1"}"#,
        r#"{"Command": "ReceiveMessage", "QueueName": "q", "VisibilityTimeout": 10}"#,
        r#"{"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": "r-1"}"#,
        r#"{"Command": "ChangeMessageVisibility", "QueueName": "q", "ReceiptHandle": "r-1", "VisibilityTimeout": 5}"#,
        r#"{"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": "r-2"}"#,
    ]);
    assert_eq!(error_code(&out[4]), "ReceiptHandleExpired");
    assert_eq!(error_code(&out[6]), "ReceiptHandleExpired");
    assert_eq!(error_code(&out[7]), "ReceiptHandleExpired");
    assert_eq!(out[8], json!({"ok": true}));
}

#[test]
fn a_handle_of_another_queue_is_expired_not_invalid() {
    let out = run(&[
        CREATE_Q,
        r#"{"Command": "CreateQueue", "QueueName": "other"}"#,
        &send("a"),
        receive(),
        r#"{"Command": "DeleteMessage", "QueueName": "other", "ReceiptHandle": "r-1"}"#,
    ]);
    assert_eq!(error_code(&out[4]), "ReceiptHandleExpired");
}

#[test]
fn a_recreated_queue_shares_nothing_with_the_deleted_one() {
    let out = run(&[
        CREATE_Q,
        &send("a"),
        receive(),
        r#"{"Command": "DeleteQueue", "QueueName": "q"}"#,
        CREATE_Q,
        &send("a"),
        r#"{"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": "r-1"}"#,
    ]);
    assert_eq!(error_code(&out[6]), "ReceiptHandleExpired");
}

#[test]
fn visibility_timeout_zero_makes_the_handle_useless() {
    let out = run(&[
        CREATE_Q,
        &send("a"),
        r#"{"Command": "ReceiveMessage", "QueueName": "q", "VisibilityTimeout": 0}"#,
        r#"{"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": "r-1"}"#,
        receive(),
    ]);
    assert_eq!(error_code(&out[3]), "ReceiptHandleExpired");
    assert_eq!(out[4]["Messages"][0]["ReceiveCount"], 2);
}

#[test]
fn change_message_visibility_replaces_the_end_of_the_timeout() {
    let out = run(&[
        CREATE_Q,
        &send("a"),
        receive(),
        &tick(10),
        r#"{"Command": "ChangeMessageVisibility", "QueueName": "q", "ReceiptHandle": "r-1", "VisibilityTimeout": 5}"#,
        &tick(4),
        receive(),
        &tick(1),
        receive(),
        r#"{"Command": "ChangeMessageVisibility", "QueueName": "q", "ReceiptHandle": "r-2", "VisibilityTimeout": 0}"#,
        receive(),
        r#"{"Command": "ChangeMessageVisibility", "QueueName": "q", "ReceiptHandle": "r-3", "VisibilityTimeout": 43201}"#,
        r#"{"Command": "ChangeMessageVisibility", "QueueName": "q", "ReceiptHandle": "r-3"}"#,
    ]);
    assert_eq!(out[4], json!({"ok": true}));
    assert_eq!(out[6]["Messages"], json!([]));
    assert_eq!(out[8]["Messages"][0]["ReceiveCount"], 2);
    assert_eq!(out[9], json!({"ok": true}));
    assert_eq!(out[10]["Messages"][0]["ReceiveCount"], 3);
    assert_eq!(error_code(&out[11]), "InvalidParameterValue");
    assert_eq!(error_code(&out[12]), "MissingParameter");
}

#[test]
fn a_message_keeps_its_place_after_its_timeout_ends() {
    let out = run(&[
        CREATE_Q,
        &send("a"),
        &send("b"),
        r#"{"Command": "ReceiveMessage", "QueueName": "q", "VisibilityTimeout": 1}"#,
        &tick(1),
        r#"{"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 2}"#,
    ]);
    let bodies: Vec<_> = out[5]["Messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["Body"].clone())
        .collect();
    assert_eq!(bodies, [json!("a"), json!("b")]);
}

#[test]
fn purge_removes_every_message_in_every_state() {
    let out = run(&[
        CREATE_Q,
        &send("a"),
        &send("b"),
        r#"{"Command": "SendMessage", "QueueName": "q", "Body": "c", "DelaySeconds": 9}"#,
        receive(),
        r#"{"Command": "PurgeQueue", "QueueName": "q"}"#,
        r#"{"Command": "GetQueueAttributes", "QueueName": "q"}"#,
        r#"{"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": "r-1"}"#,
        &send("d"),
    ]);
    assert_eq!(out[5], json!({"ok": true}));
    assert_eq!(
        counts(&out[6]),
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(error_code(&out[7]), "ReceiptHandleExpired");
    assert_eq!(out[8]["MessageId"], "m-4");
}

#[test]
fn retention_removes_messages_in_every_state_at_once() {
    let out = run(&[
        r#"{"Command": "CreateQueue", "QueueName": "q", "Attributes": {"RetentionSeconds": 60}}"#,
        &send("old"),
        r#"{"Command": "SendMessage", "QueueName": "q", "Body": "delayed", "DelaySeconds": 900}"#,
        &tick(30),
        &send("young"),
        r#"{"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 2}"#,
        &tick(29),
        r#"{"Command": "GetQueueAttributes", "QueueName": "q"}"#,
        &tick(1),
        r#"{"Command": "GetQueueAttributes", "QueueName": "q"}"#,
        r#"{"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": "r-1"}"#,
        &tick(30),
        r#"{"Command": "GetQueueAttributes", "QueueName": "q"}"#,
    ]);
    assert_eq!(
        counts(&out[7]),
        json!({"Visible": 0, "InFlight": 2, "Delayed": 1})
    );
    // At t=60 "old" and "delayed" are gone. "young" (sent at 30) stays, and its
    // 30 s visibility timeout from t=30 has just ended.
    assert_eq!(
        counts(&out[9]),
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(error_code(&out[10]), "ReceiptHandleExpired");
    assert_eq!(
        counts(&out[12]),
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn lowering_retention_removes_at_once_and_raising_it_does_not_bring_messages_back() {
    let out = run(&[
        CREATE_Q,
        &send("a"),
        &tick(100),
        r#"{"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"RetentionSeconds": 100}}"#,
        r#"{"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"RetentionSeconds": 1000}}"#,
        r#"{"Command": "GetQueueAttributes", "QueueName": "q"}"#,
        &send("b"),
        &tick(99),
        r#"{"Command": "GetQueueAttributes", "QueueName": "q"}"#,
    ]);
    assert_eq!(
        counts(&out[5]),
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(
        counts(&out[8]),
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn a_queue_holds_at_most_100000_messages() {
    let mut commands = vec![CREATE_Q.to_string()];
    commands.extend((0..100_000).map(|_| send("x")));
    commands.push(send("one too many"));
    commands.push(r#"{"Command": "PurgeQueue", "QueueName": "q"}"#.into());
    commands.push(send("fits again"));
    let refs: Vec<&str> = commands.iter().map(String::as_str).collect();
    let out = run(&refs);
    assert_eq!(out[100_000]["MessageId"], "m-100000");
    assert_eq!(error_code(&out[100_001]), "LimitExceeded");
    assert_eq!(out[100_003]["MessageId"], "m-100001");
}
