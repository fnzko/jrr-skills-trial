//! End-to-end: run the real `miniq` binary with piped stdin and check stdout.

use std::io::Write;
use std::process::{Command, Stdio};

use serde_json::{json, Value};

/// Runs the binary on `input` and returns its stdout lines, parsed, plus exit status.
fn run(input: &[u8]) -> (Vec<Value>, bool) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_miniq"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn miniq");
    child.stdin.take().unwrap().write_all(input).unwrap();
    let output = child.wait_with_output().unwrap();
    let lines = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).expect("each stdout line is JSON"))
        .collect();
    (lines, output.status.success())
}

#[test]
fn a_session_answers_each_line_in_order_and_exits_zero() {
    let input = concat!(
        r#"{"Command": "CreateQueue", "QueueName": "jobs"}"#,
        "\n",
        r#"{"Command": "SendMessage", "QueueName": "jobs", "Body": "build"}"#,
        "\n",
        r#"{"Command": "ReceiveMessage", "QueueName": "jobs"}"#,
        "\n",
        r#"{"Command": "DeleteMessage", "QueueName": "jobs", "ReceiptHandle": "r-1"}"#,
        "\n",
        r#"{"Command": "ReceiveMessage", "QueueName": "jobs"}"#,
        "\n",
    );
    let (lines, success) = run(input.as_bytes());
    assert!(success);
    assert_eq!(lines.len(), 5);
    assert_eq!(lines[0], json!({"ok": true}));
    assert_eq!(lines[1], json!({"ok": true, "MessageId": "m-1"}));
    assert_eq!(lines[2]["Messages"][0]["Body"], json!("build"));
    assert_eq!(lines[3], json!({"ok": true}));
    assert_eq!(lines[4], json!({"ok": true, "Messages": []}));
}

#[test]
fn errors_are_responses_not_exit_codes() {
    let (lines, success) = run(b"{\"Command\": \"DeleteQueue\", \"QueueName\": \"nope\"}\n");
    assert!(success);
    assert_eq!(
        lines,
        vec![json!({
            "ok": false,
            "error": {"code": "QueueDoesNotExist", "message": "no queue named \"nope\""}
        })]
    );
}

#[test]
fn empty_input_prints_nothing_and_exits_zero() {
    let (lines, success) = run(b"");
    assert!(success);
    assert!(lines.is_empty());
}

#[test]
fn a_final_line_without_newline_is_answered() {
    let (lines, _) = run(b"{\"Command\": \"Tick\", \"Seconds\": 9}");
    assert_eq!(lines, vec![json!({"ok": true, "Now": 9})]);
}

#[test]
fn state_is_per_process() {
    // A second process starts with an empty broker: the clock is back at 0.
    let (first, _) = run(b"{\"Command\": \"Tick\", \"Seconds\": 5}\n");
    let (second, _) = run(b"{\"Command\": \"Tick\", \"Seconds\": 0}\n");
    assert_eq!(first[0]["Now"], json!(5));
    assert_eq!(second[0]["Now"], json!(0));
}
