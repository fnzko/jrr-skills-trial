use std::io::Write;
use std::process::{Command, Stdio};

use serde::Deserialize;

#[derive(Deserialize)]
struct Reply {
    ok: bool,
    error: ErrorBody,
}

#[derive(Deserialize)]
struct ErrorBody {
    code: String,
}

#[test]
fn every_line_gets_one_response_and_a_last_line_without_newline_counts() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_miniq"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("the miniq binary starts");
    child
        .stdin
        .take()
        .expect("stdin is piped")
        .write_all(b"{\"Command\":\"Tick\",\"Seconds\":1}\n\n{\"Command\":\"Tick\",\"Seconds\":2}")
        .expect("input is written");
    let output = child
        .wait_with_output()
        .expect("miniq exits at end of input");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("responses are UTF-8");
    assert!(stdout.ends_with('\n'));
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[0], r#"{"ok":true,"Now":1}"#);
    let failure: Reply = serde_json::from_str(lines[1]).expect("the blank line gets a failure");
    assert!(!failure.ok);
    assert_eq!(failure.error.code, "InvalidRequest");
    assert_eq!(lines[2], r#"{"ok":true,"Now":3}"#);
}
