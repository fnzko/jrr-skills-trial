//! End-to-end tests of the binary: lines go in on stdin and responses come out on stdout.

use serde_json::{json, Value};
use std::io::Write;
use std::process::{Command, Stdio};

/// Runs miniq with `input` on stdin and returns each stdout line parsed as JSON.
fn run(input: &'static str) -> Vec<Value> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_miniq"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("miniq starts");
    let mut stdin = child.stdin.take().expect("stdin is piped");
    // Written from another thread so that a large response cannot block the input.
    let writer = std::thread::spawn(move || stdin.write_all(input.as_bytes()));
    let output = child.wait_with_output().expect("miniq finishes");
    writer
        .join()
        .expect("writer thread does not panic")
        .expect("input is written");
    assert!(output.status.success(), "exit status {:?}", output.status);
    let stdout = String::from_utf8(output.stdout).expect("stdout is UTF-8");
    stdout
        .lines()
        .map(|line| serde_json::from_str(line).expect("each response line is JSON"))
        .collect()
}

#[test]
fn each_line_gets_one_response_in_order_including_a_final_line_without_newline() {
    let replies = run(concat!(
        r#"{"Command":"CreateQueue","QueueName":"q"}"#,
        "\n",
        "\n",
        "[1]\n",
        r#"{"Command":"SendMessage","QueueName":"q","Body":"hi"}"#,
    ));
    assert_eq!(replies.len(), 4);
    assert_eq!(replies[0], json!({"ok": true}));
    assert_eq!(replies[1]["ok"], false);
    assert_eq!(replies[1]["error"]["code"], "InvalidRequest");
    assert_eq!(replies[2]["error"]["code"], "InvalidRequest");
    assert_eq!(replies[3], json!({"ok": true, "MessageId": "m-1"}));
}

#[test]
fn empty_input_gives_no_output_and_exit_status_zero() {
    assert_eq!(run(""), Vec::<Value>::new());
}
