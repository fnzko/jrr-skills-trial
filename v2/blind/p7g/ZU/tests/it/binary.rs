use std::io::Write;
use std::process::{Command, Stdio};

use serde_json::json;

#[test]
fn the_binary_answers_each_line_on_stdout_and_exits_with_zero() {
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
        .write_all(b"{\"Command\":\"Tick\",\"Seconds\":2}\n\n{\"Command\":\"Tick\",\"Seconds\":3}")
        .expect("input is written");
    let output = child.wait_with_output().expect("the process finishes");
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stderr, b"");
    let stdout = String::from_utf8(output.stdout).expect("stdout is UTF-8");
    let lines: Vec<serde_json::Value> = stdout
        .lines()
        .map(|line| serde_json::from_str(line).expect("each line is JSON"))
        .collect();
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[0], json!({"ok": true, "Now": 2}));
    assert_eq!(lines[1]["error"]["code"], json!("InvalidRequest"));
    assert_eq!(lines[2], json!({"ok": true, "Now": 5}));
    assert!(stdout.ends_with('\n'));
}
