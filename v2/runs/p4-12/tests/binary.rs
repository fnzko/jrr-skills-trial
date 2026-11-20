//! Runs the real `miniq` binary over stdin/stdout.

use std::io::Write;
use std::process::{Command, Stdio};

fn run_miniq(input: &[u8]) -> (String, i32) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_miniq"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("miniq starts");
    child.stdin.take().unwrap().write_all(input).unwrap();
    let output = child.wait_with_output().unwrap();
    (
        String::from_utf8(output.stdout).expect("stdout is UTF-8"),
        output.status.code().expect("miniq exits normally"),
    )
}

#[test]
fn answers_every_line_in_order_and_exits_zero() {
    let input = concat!(
        "{\"Command\": \"CreateQueue\", \"QueueName\": \"jobs\"}\n",
        "\n",
        "{\"Command\": \"SendMessage\", \"QueueName\": \"jobs\", \"Body\": \"hi\"}\r\n",
        "{\"Command\": \"Tick\", \"Seconds\": 4}", // Final line without a newline still counts.
    );
    let (stdout, code) = run_miniq(input.as_bytes());
    assert_eq!(code, 0);
    let lines: Vec<serde_json::Value> = stdout
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(lines.len(), 4);
    assert_eq!(lines[0], serde_json::json!({"ok": true}));
    assert_eq!(lines[1]["error"]["code"], "InvalidRequest");
    assert_eq!(lines[2]["MessageId"], "m-1");
    assert_eq!(lines[3], serde_json::json!({"ok": true, "Now": 4}));
    assert!(stdout.ends_with('\n'));
}

#[test]
fn empty_input_produces_no_output() {
    assert_eq!(run_miniq(b""), (String::new(), 0));
}
