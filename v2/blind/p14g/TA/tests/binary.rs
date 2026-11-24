//! The `miniq` executable reads stdin, writes only responses to stdout and exits with 0.

use std::io::Write;
use std::process::{Command, Stdio};

#[test]
fn binary_answers_each_line_and_exits_zero_at_end_of_input() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_miniq"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the miniq binary starts");
    child
        .stdin
        .take()
        .expect("stdin is piped")
        .write_all(b"{\"Command\":\"Tick\",\"Seconds\":4}\n\n{\"Command\":\"Tick\",\"Seconds\":6}")
        .expect("miniq reads its input");
    let output = child.wait_with_output().expect("miniq exits");

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stderr, b"");
    let stdout = String::from_utf8(output.stdout).expect("stdout is UTF-8");
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[0], r#"{"ok":true,"Now":4}"#);
    assert!(lines[1].starts_with(r#"{"ok":false,"error":{"code":"InvalidRequest""#));
    assert_eq!(lines[2], r#"{"ok":true,"Now":10}"#);
    assert!(stdout.ends_with('\n'));
}
