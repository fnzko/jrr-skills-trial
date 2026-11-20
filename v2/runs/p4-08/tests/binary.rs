use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

use serde_json::{Value, json};

/// Runs the real binary on `input` and returns its exit success and stdout lines.
fn run(input: &[u8]) -> (bool, Vec<Value>) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_miniq"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input).unwrap();
    let output = child.wait_with_output().unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.ends_with('\n'), "stdout {stdout:?}");
    let lines = stdout
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    (output.status.success(), lines)
}

fn without_message(mut response: Value) -> Value {
    if let Some(error) = response.get_mut("error") {
        error.as_object_mut().unwrap().remove("message").unwrap();
    }
    response
}

#[test]
fn answers_every_line_in_order_and_exits_zero() {
    let input = b"{\"Command\": \"CreateQueue\", \"QueueName\": \"q\"}\n\
        \n\
        \xff\xfe\n\
        {\"Command\": \"SendMessage\", \"QueueName\": \"q\", \"Body\": \"hi\"}\n\
        {\"Command\": \"Tick\", \"Seconds\": 4}";
    let (success, lines) = run(input);
    assert!(success);
    let lines: Vec<Value> = lines.into_iter().map(without_message).collect();
    assert_eq!(
        lines,
        [
            json!({"ok": true}),
            json!({"ok": false, "error": {"code": "InvalidRequest"}}),
            json!({"ok": false, "error": {"code": "InvalidRequest"}}),
            json!({"ok": true, "MessageId": "m-1"}),
            json!({"ok": true, "Now": 4}),
        ]
    );
}

#[test]
fn empty_input_gives_no_output() {
    let child = Command::new(env!("CARGO_BIN_EXE_miniq"))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    assert_eq!(output.stdout, b"");
}

#[test]
fn answers_each_line_before_the_next_one_arrives() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_miniq"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    for (seconds, now) in [(2, 2), (3, 5)] {
        writeln!(stdin, r#"{{"Command": "Tick", "Seconds": {seconds}}}"#).unwrap();
        line.clear();
        // Blocks forever if the response is not flushed while stdin stays open.
        stdout.read_line(&mut line).unwrap();
        let response: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(response, json!({"ok": true, "Now": now}));
    }
    drop(stdin);
    assert!(child.wait().unwrap().success());
}
