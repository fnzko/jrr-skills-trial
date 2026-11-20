//! The binary's line framing.

use std::io::Write;
use std::process::{Command, Stdio};

fn run(input: &[u8]) -> (String, bool) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_miniq"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn miniq");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(input)
        .expect("write");
    let output = child.wait_with_output().expect("wait");
    (
        String::from_utf8(output.stdout).expect("utf8"),
        output.status.success(),
    )
}

#[test]
fn every_line_gets_one_response_and_a_last_line_without_newline_counts() {
    let (out, ok) =
        run(b"{\"Command\":\"Tick\",\"Seconds\":2}\n\n{\"Command\":\"Tick\",\"Seconds\":3}");
    assert!(ok);
    let lines: Vec<_> = out.lines().collect();
    assert_eq!(lines.len(), 3, "{out}");
    assert_eq!(lines[0], r#"{"ok":true,"Now":2}"#);
    assert!(lines[1].contains("InvalidRequest"));
    assert_eq!(lines[2], r#"{"ok":true,"Now":5}"#);
}

#[test]
fn invalid_utf8_gets_an_error_response_and_processing_continues() {
    let (out, ok) = run(b"\xff\xfe\n{\"Command\":\"ListQueues\"}\n");
    assert!(ok);
    let lines: Vec<_> = out.lines().collect();
    assert_eq!(lines.len(), 2);
    assert!(lines[0].contains("InvalidRequest"));
    assert_eq!(lines[1], r#"{"ok":true,"QueueNames":[]}"#);
}

#[test]
fn empty_input_produces_no_output() {
    assert_eq!(run(b""), (String::new(), true));
}
