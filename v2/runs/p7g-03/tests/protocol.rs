//! End-to-end tests of the line protocol through the real binary.

use std::io::Write;
use std::process::{Command, Stdio};

fn run(input: &[u8]) -> Vec<String> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_miniq"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input).unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success());
    String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect()
}

#[test]
fn every_line_gets_one_response_in_order() {
    let lines =
        run(b"{\"Command\":\"Tick\",\"Seconds\":5}\n\n{\"Command\":\"Tick\",\"Seconds\":1}\n");
    assert_eq!(lines.len(), 3);
    assert!(lines[0].contains("\"Now\":5"));
    assert!(lines[1].contains("InvalidRequest"));
    assert!(lines[2].contains("\"Now\":6"));
}

#[test]
fn final_line_without_newline_counts() {
    assert_eq!(run(b"{\"Command\":\"Tick\",\"Seconds\":1}").len(), 1);
    assert!(run(b"").is_empty());
}

#[test]
fn invalid_utf8_is_an_invalid_request_and_processing_continues() {
    let lines = run(b"\xff\xfe\n{\"Command\":\"Tick\",\"Seconds\":2}\n");
    assert!(lines[0].contains("InvalidRequest"));
    assert!(lines[1].contains("\"Now\":2"));
}
