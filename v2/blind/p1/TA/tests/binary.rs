//! Tests of the real `miniq` process: framing, flushing and exit status.

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

use serde_json::{Value, json};

fn parse_line(line: &str) -> Value {
    serde_json::from_str(line).unwrap()
}

fn miniq() -> Command {
    Command::new(env!("CARGO_BIN_EXE_miniq"))
}

#[test]
fn final_line_without_newline_counts_and_exit_status_is_zero() {
    let mut child = miniq()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"{\"Command\":\"Tick\",\"Seconds\":2}\n\n{\"Command\":\"Tick\",\"Seconds\":3}")
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 3);
    assert_eq!(parse_line(lines[0]), json!({"ok": true, "Now": 2}));
    assert_eq!(parse_line(lines[1])["error"]["code"], "InvalidRequest");
    assert_eq!(parse_line(lines[2]), json!({"ok": true, "Now": 5}));
    assert!(stdout.ends_with('\n'));
}

#[test]
fn invalid_utf8_is_one_failed_command_not_the_end_of_input() {
    let mut child = miniq()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(
            b"{\"Command\":\"Tick\",\"Seconds\":\xff}\n{\"Command\":\"Tick\",\"Seconds\":1}\n",
        )
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 2);
    assert_eq!(parse_line(lines[0])["error"]["code"], "InvalidRequest");
    assert_eq!(parse_line(lines[1]), json!({"ok": true, "Now": 1}));
}

#[test]
fn each_response_is_flushed_before_the_next_line_is_read() {
    let mut child = miniq()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());

    stdin
        .write_all(b"{\"Command\":\"Tick\",\"Seconds\":1}\n")
        .unwrap();
    let mut response = String::new();
    stdout.read_line(&mut response).unwrap();
    assert_eq!(parse_line(&response), json!({"ok": true, "Now": 1}));

    stdin
        .write_all(b"{\"Command\":\"Tick\",\"Seconds\":1}\n")
        .unwrap();
    response.clear();
    stdout.read_line(&mut response).unwrap();
    assert_eq!(parse_line(&response), json!({"ok": true, "Now": 2}));

    drop(stdin);
    assert!(child.wait().unwrap().success());
}
