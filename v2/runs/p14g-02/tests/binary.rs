//! End-to-end checks of the `miniq` process: framing of stdin and stdout.

use std::io::Write;
use std::process::{Command, Stdio};

use serde_json::{Value, json};

/// Runs miniq on `input` and returns its stdout lines, parsed, and whether
/// it exited with 0.
fn run(input: &[u8]) -> (Vec<Value>, bool) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_miniq"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("the miniq binary starts");
    child
        .stdin
        .take()
        .expect("stdin is piped")
        .write_all(input)
        .expect("the input fits the pipe buffer");
    let output = child
        .wait_with_output()
        .expect("miniq exits at end of input");
    let stdout = String::from_utf8(output.stdout).expect("responses are UTF-8");
    assert!(stdout.is_empty() || stdout.ends_with('\n'));
    let responses = stdout
        .lines()
        .map(|line| serde_json::from_str(line).expect("each line is one JSON value"))
        .collect();
    (responses, output.status.success())
}

fn error_code(response: &Value) -> &Value {
    &response["error"]["code"]
}

#[test]
fn every_line_gets_one_response_in_order() {
    let (responses, success) = run(
        b"{\"Command\": \"Tick\", \"Seconds\": 2}\n\nnot json\n{\"Command\": \"Tick\", \"Seconds\": 3}\n",
    );
    assert!(success);
    assert_eq!(responses.len(), 4);
    assert_eq!(responses[0], json!({"ok": true, "Now": 2}));
    assert_eq!(error_code(&responses[1]), "InvalidRequest");
    assert_eq!(error_code(&responses[2]), "InvalidRequest");
    assert_eq!(responses[3], json!({"ok": true, "Now": 5}));
}

#[test]
fn a_final_line_without_newline_counts() {
    let (responses, success) = run(b"{\"Command\": \"Tick\", \"Seconds\": 4}");
    assert!(success);
    assert_eq!(responses, [json!({"ok": true, "Now": 4})]);
}

#[test]
fn empty_input_produces_no_output() {
    assert_eq!(run(b""), (vec![], true));
}

#[test]
fn invalid_utf8_is_answered_and_the_next_line_still_runs() {
    let (responses, success) = run(b"\xff\xfe\n{\"Command\": \"Tick\", \"Seconds\": 1}\n");
    assert!(success);
    assert_eq!(responses.len(), 2);
    assert_eq!(error_code(&responses[0]), "InvalidRequest");
    assert_eq!(responses[1], json!({"ok": true, "Now": 1}));
}
