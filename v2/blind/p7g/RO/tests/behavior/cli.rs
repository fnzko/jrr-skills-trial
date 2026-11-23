use std::io::Write;
use std::process::{Command, Stdio};

use serde_json::json;

#[test]
fn the_binary_answers_each_line_and_exits_zero_at_end_of_input() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_miniq"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("the test binary can be started");
    child
        .stdin
        .take()
        .expect("stdin is piped")
        .write_all(
            b"{\"Command\": \"Tick\", \"Seconds\": 3}\n{\"Command\": \"Tick\", \"Seconds\": 4}",
        )
        .expect("the child reads its input");
    let output = child.wait_with_output().expect("the child exits");
    assert!(output.status.success());
    let lines: Vec<serde_json::Value> = String::from_utf8(output.stdout)
        .expect("output is UTF-8")
        .lines()
        .map(|line| serde_json::from_str(line).expect("one JSON object per line"))
        .collect();
    assert_eq!(
        lines,
        [json!({"ok": true, "Now": 3}), json!({"ok": true, "Now": 7})]
    );
}
