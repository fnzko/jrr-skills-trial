//! Tests of the `miniq` executable itself: framing on stdin and stdout.

use std::io::Write;
use std::process::{Command, Stdio};

use serde_json::{Value, json};

fn parse(line: &str) -> Value {
    serde_json::from_str(line).expect("miniq writes one JSON object per line")
}

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
    assert_eq!(parse(&lines[0]), json!({"ok": true, "Now": 1}));
    assert_eq!(parse(&lines[1])["error"]["code"], "InvalidRequest");
    assert_eq!(parse(&lines[2])["error"]["code"], "InvalidRequest");
    assert_eq!(parse(&lines[3]), json!({"ok": true, "Now": 3}));
}

#[test]
fn final_line_without_newline_counts_if_not_empty() {
    let (lines, ok) = run(b"{\"Command\":\"Tick\",\"Seconds\":4}");
    assert!(ok);
    assert_eq!(lines.len(), 1);
    assert_eq!(parse(&lines[0]), json!({"ok": true, "Now": 4}));

    let (lines, ok) = run(b"");
    assert!(ok);
    assert_eq!(lines, Vec::<String>::new());

    // A trailing empty fragment is not a line, but a blank line is.
    let (lines, ok) = run(b"\n");
    assert!(ok);
    assert_eq!(lines.len(), 1);
    assert_eq!(parse(&lines[0])["error"]["code"], "InvalidRequest");
}

#[test]
fn crlf_line_endings_are_json_whitespace() {
    let (lines, _) = run(b"{\"Command\":\"Tick\",\"Seconds\":1}\r\n");
    assert_eq!(parse(&lines[0]), json!({"ok": true, "Now": 1}));
}
