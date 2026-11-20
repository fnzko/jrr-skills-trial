//! Runs the `miniq` binary as a client would, through pipes.

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Command, Stdio};

use serde_json::{json, Value};

fn spawn() -> std::process::Child {
    Command::new(env!("CARGO_BIN_EXE_miniq"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("miniq binary starts")
}

#[test]
fn answers_each_line_before_the_client_sends_the_next() {
    let mut child = spawn();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());

    // Each response must arrive before the next line is written: a client that
    // waits for an answer would otherwise hang.
    writeln!(
        stdin,
        r#"{{"Command": "CreateQueue", "QueueName": "jobs"}}"#
    )
    .unwrap();
    stdin.flush().unwrap();
    let mut line = String::new();
    stdout.read_line(&mut line).unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&line).unwrap(),
        json!({"ok": true})
    );

    writeln!(stdin, r#"{{"Command": "Tick", "Seconds": 3}}"#).unwrap();
    stdin.flush().unwrap();
    line.clear();
    stdout.read_line(&mut line).unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&line).unwrap(),
        json!({"ok": true, "Now": 3})
    );

    drop(stdin);
    let mut rest = String::new();
    stdout.read_to_string(&mut rest).unwrap();
    assert_eq!(rest, "");
    assert!(child.wait().unwrap().success());
}

#[test]
fn bad_lines_get_error_responses_and_nothing_goes_to_stderr() {
    let mut child = spawn();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"not json\n\n{\"Command\": \"Tick\", \"Seconds\": 1}")
        .unwrap();

    let output = child.wait_with_output().unwrap();

    assert!(output.status.success());
    assert!(
        output.stderr.is_empty(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let responses: Vec<Value> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(responses.len(), 3);
    assert_eq!(responses[0]["error"]["code"], "InvalidRequest");
    assert_eq!(responses[1]["error"]["code"], "InvalidRequest");
    assert_eq!(responses[2], json!({"ok": true, "Now": 1}));
}
