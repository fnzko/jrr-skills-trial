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
    assert!(lines[0].contains("\"Now\":1"));
    assert!(lines[1].contains("InvalidRequest"));
    assert!(lines[2].contains("InvalidRequest"));
    assert!(lines[3].contains("\"Now\":3"));
}

#[test]
fn final_line_without_newline_counts_if_not_empty() {
    let (lines, ok) = run(b"{\"Command\":\"Tick\",\"Seconds\":4}");
    assert!(ok);
    assert_eq!(lines.len(), 1);
    assert!(lines[0].contains("\"Now\":4"));

    let (lines, ok) = run(b"");
    assert!(ok && lines.is_empty());

    // A trailing empty fragment is not a line, but a blank line is.
    let (lines, _) = run(b"\n");
    assert_eq!(lines.len(), 1);
}

#[test]
fn crlf_line_endings_are_json_whitespace() {
    let (lines, _) = run(b"{\"Command\":\"Tick\",\"Seconds\":1}\r\n");
    assert!(lines[0].contains("\"Now\":1"));
}
