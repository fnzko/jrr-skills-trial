//! Runs the real binary to check the line protocol: one response per line, in order.

use std::io::Write;
use std::process::{Command, Stdio};

use serde_json::{Value, json};

fn run_miniq(input: &[u8]) -> (String, Option<i32>) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_miniq"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("miniq starts");
    child
        .stdin
        .take()
        .expect("stdin is piped")
        .write_all(input)
        .expect("write input");
    let output = child.wait_with_output().expect("miniq exits");
    (
        String::from_utf8(output.stdout).expect("stdout is UTF-8"),
        output.status.code(),
    )
}

fn lines(stdout: &str) -> Vec<Value> {
    stdout
        .lines()
        .map(|line| serde_json::from_str(line).expect("each response line is JSON"))
        .collect()
}

#[test]
fn every_line_gets_one_response_in_order_and_the_exit_code_is_zero() {
    let (stdout, code) = run_miniq(
        b"{\"Command\":\"Tick\",\"Seconds\":2}\n\n{\"Command\":\"Tick\",\"Seconds\":3}\n",
    );
    assert_eq!(code, Some(0));
    assert!(stdout.ends_with('\n'));
    let responses = lines(&stdout);
    assert_eq!(responses.len(), 3);
    assert_eq!(responses[0], json!({"ok": true, "Now": 2}));
    assert_eq!(responses[1]["error"]["code"], json!("InvalidRequest"));
    assert_eq!(responses[2], json!({"ok": true, "Now": 5}));
}

#[test]
fn a_final_line_without_a_newline_counts() {
    let (stdout, code) = run_miniq(b"{\"Command\":\"Tick\",\"Seconds\":2}");
    assert_eq!(code, Some(0));
    assert_eq!(lines(&stdout), [json!({"ok": true, "Now": 2})]);
}

#[test]
fn empty_input_produces_no_output() {
    assert_eq!(run_miniq(b""), (String::new(), Some(0)));
}

#[test]
fn invalid_utf8_is_one_failed_command_and_processing_carries_on() {
    let (stdout, code) = run_miniq(
        b"{\"Command\":\"Tick\",\"Seconds\":1}\xff\n{\"Command\":\"Tick\",\"Seconds\":1}\n",
    );
    assert_eq!(code, Some(0));
    let responses = lines(&stdout);
    assert_eq!(responses[0]["error"]["code"], json!("InvalidRequest"));
    assert_eq!(responses[1], json!({"ok": true, "Now": 1}));
}

#[test]
fn crlf_line_endings_are_json_whitespace() {
    let (stdout, _) = run_miniq(b"{\"Command\":\"Tick\",\"Seconds\":1}\r\n");
    assert_eq!(lines(&stdout), [json!({"ok": true, "Now": 1})]);
}
