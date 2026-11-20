//! Framing of the `miniq` binary: one response line per input line.

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

fn codes(stdout: &str) -> Vec<String> {
    stdout
        .lines()
        .map(|line| {
            let response: serde_json::Value = serde_json::from_str(line).unwrap();
            response["error"]["code"].as_str().unwrap().to_owned()
        })
        .collect()
}

#[test]
fn every_line_gets_one_response_including_blank_ones() {
    let (stdout, success) = run(b"\n{}\n\nx\n");
    assert!(success);
    assert_eq!(codes(&stdout), ["InvalidRequest"; 4]);
}

#[test]
fn final_line_without_newline_counts() {
    let (stdout, success) = run(b"{}\n{}");
    assert!(success);
    assert_eq!(codes(&stdout), ["InvalidRequest"; 2]);
}

#[test]
fn empty_input_produces_no_output() {
    let (stdout, success) = run(b"");
    assert!(success);
    assert_eq!(stdout, "");
}

#[test]
fn invalid_utf8_line_does_not_stop_the_service() {
    let (stdout, success) = run(b"\xff\xfe\n{}\n");
    assert!(success);
    assert_eq!(codes(&stdout), ["InvalidRequest"; 2]);
}
