//! Process-level tests: framing, flushing and exit status of the binary.

use std::io::{Read, Write};
use std::process::{Command, Stdio};

fn run(input: &[u8]) -> (Vec<String>, i32) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_miniq"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input).unwrap();
    let mut out = String::new();
    child
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut out)
        .unwrap();
    let status = child.wait().unwrap();
    assert!(out.is_empty() || out.ends_with('\n'));
    (
        out.lines().map(str::to_owned).collect(),
        status.code().unwrap(),
    )
}

#[test]
fn one_response_per_line_including_blank_and_unterminated() {
    let (out, code) =
        run(b"{\"Command\":\"Tick\",\"Seconds\":5}\n\n{\"Command\":\"Tick\",\"Seconds\":1}");
    assert_eq!(code, 0);
    assert_eq!(out.len(), 3);
    assert!(out[0].contains("\"Now\":5"));
    assert!(out[1].contains("InvalidRequest"));
    assert!(out[2].contains("\"Now\":6"));
}

#[test]
fn invalid_utf8_is_answered_and_the_next_line_still_runs() {
    let (out, code) = run(b"\xff\xfe\n{\"Command\":\"ListQueues\"}\n");
    assert_eq!(code, 0);
    assert!(out[0].contains("InvalidRequest"));
    assert!(out[1].contains("\"ok\":true"));
}

#[test]
fn empty_input_gives_no_output() {
    assert_eq!(run(b""), (vec![], 0));
}
