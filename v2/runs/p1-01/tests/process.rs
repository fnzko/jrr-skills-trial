//! Runs the real binary to check the line protocol on stdin and stdout.

use std::io::Write;
use std::process::{Command, Stdio};

fn run(input: &[u8]) -> (String, bool) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_miniq"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input).unwrap();
    let output = child.wait_with_output().unwrap();
    (
        String::from_utf8(output.stdout).unwrap(),
        output.status.success(),
    )
}

#[test]
fn every_line_gets_exactly_one_response_in_order_and_the_exit_code_is_zero() {
    let input = concat!(
        "{\"Command\": \"Tick\", \"Seconds\": 2}\n",
        "\n",
        "not json\n",
        "{\"Command\": \"Tick\", \"Seconds\": 3}\n",
    );
    let (stdout, success) = run(input.as_bytes());
    let lines: Vec<&str> = stdout.lines().collect();
    assert!(success);
    assert_eq!(lines.len(), 4);
    assert_eq!(lines[0], r#"{"ok":true,"Now":2}"#);
    assert!(
        lines[1].contains(r#""code":"InvalidRequest""#),
        "{}",
        lines[1]
    );
    assert!(
        lines[2].contains(r#""code":"InvalidRequest""#),
        "{}",
        lines[2]
    );
    assert_eq!(lines[3], r#"{"ok":true,"Now":5}"#);
    assert!(stdout.ends_with('\n'));
}

#[test]
fn a_final_line_without_a_newline_counts() {
    let (stdout, success) = run(br#"{"Command": "Tick", "Seconds": 1}"#);
    assert!(success);
    assert_eq!(stdout, "{\"ok\":true,\"Now\":1}\n");
}

#[test]
fn empty_input_produces_no_output() {
    let (stdout, success) = run(b"");
    assert!(success);
    assert_eq!(stdout, "");
}

#[test]
fn invalid_utf8_is_an_invalid_request_and_the_next_line_still_runs() {
    let mut input = b"{\"Command\": \"Tick\", \"Seconds\": 1, \"X\": \"\xff\"}\n".to_vec();
    input.extend_from_slice(b"{\"Command\": \"Tick\", \"Seconds\": 1}\n");
    let (stdout, success) = run(&input);
    let lines: Vec<&str> = stdout.lines().collect();
    assert!(success);
    assert_eq!(lines.len(), 2);
    assert!(
        lines[0].contains(r#""code":"InvalidRequest""#),
        "{}",
        lines[0]
    );
    assert_eq!(lines[1], r#"{"ok":true,"Now":1}"#);
}

#[test]
fn a_blank_final_line_is_a_command_but_the_empty_remainder_after_the_last_newline_is_not() {
    let (stdout, _) = run(b"\n");
    assert_eq!(stdout.lines().count(), 1);
    let (stdout, _) = run(b"   ");
    assert_eq!(stdout.lines().count(), 1);
}
