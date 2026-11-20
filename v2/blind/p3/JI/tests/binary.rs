//! Tests of the `miniq` executable itself: framing on stdin and stdout.

use std::io::Write;
use std::process::{Command, Stdio};

fn run(input: &[u8]) -> (Vec<String>, bool) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_miniq"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input).unwrap();
    let output = child.wait_with_output().unwrap();
    let lines = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect();
    (lines, output.status.success())
}

#[test]
fn every_line_gets_one_response_in_order() {
    let (lines, ok) = run(
        b"{\"Command\":\"Tick\",\"Seconds\":1}\n\n\xff\n{\"Command\":\"Tick\",\"Seconds\":2}\n",
    );
    assert!(ok);
    assert_eq!(lines.len(), 4);
    assert_eq!(lines[0], r#"{"Now":1,"ok":true}"#);
    for line in &lines[1..=2] {
        assert!(line.starts_with(r#"{"error":{"code":"InvalidRequest","#), "{line}");
    }
    assert_eq!(lines[3], r#"{"Now":3,"ok":true}"#);
}

#[test]
fn final_line_without_newline_counts_if_not_empty() {
    let (lines, ok) = run(b"{\"Command\":\"Tick\",\"Seconds\":4}");
    assert!(ok);
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0], r#"{"Now":4,"ok":true}"#);

    let (lines, ok) = run(b"");
    assert!(ok && lines.is_empty());

    // A trailing empty fragment is not a line, but a blank line is.
    let (lines, _) = run(b"\n");
    assert_eq!(lines.len(), 1);
}

#[test]
fn crlf_line_endings_are_json_whitespace() {
    let (lines, _) = run(b"{\"Command\":\"Tick\",\"Seconds\":1}\r\n");
    assert_eq!(lines[0], r#"{"Now":1,"ok":true}"#);
}
