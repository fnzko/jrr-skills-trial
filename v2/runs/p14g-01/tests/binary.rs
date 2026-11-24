//! The `miniq` binary speaks the protocol on stdin and stdout and exits with 0 at end of input.

use std::io::Write;
use std::process::{Command, Stdio};

#[test]
fn binary_answers_each_line_and_exits_cleanly() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_miniq"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("the miniq binary starts");
    child
        .stdin
        .take()
        .expect("stdin is piped")
        .write_all(b"{\"Command\":\"Tick\",\"Seconds\":7}\n\n{\"Command\":\"Tick\",\"Seconds\":1}")
        .expect("miniq reads stdin");
    let output = child
        .wait_with_output()
        .expect("miniq exits at end of input");
    assert_eq!(output.status.code(), Some(0));
    let lines: Vec<serde_json::Value> = String::from_utf8(output.stdout)
        .expect("stdout is UTF-8")
        .lines()
        .map(|line| serde_json::from_str(line).expect("each line is JSON"))
        .collect();
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[0], serde_json::json!({"ok": true, "Now": 7}));
    assert_eq!(lines[1]["error"]["code"], "InvalidRequest");
    assert_eq!(lines[2], serde_json::json!({"ok": true, "Now": 8}));
}
