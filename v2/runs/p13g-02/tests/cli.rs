//! Runs the real binary to check the line protocol on stdin and stdout.

use std::io::Write;
use std::process::{Command, Stdio};
use std::thread;

fn run_miniq(input: &[u8]) -> (String, Option<i32>) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_miniq"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("the miniq binary starts");
    let mut stdin = child.stdin.take().expect("stdin is piped");
    let input = input.to_vec();
    // Feeding stdin from its own thread keeps a full stdout pipe from blocking both sides.
    let writer = thread::spawn(move || stdin.write_all(&input));
    let output = child.wait_with_output().expect("miniq exits");
    writer
        .join()
        .expect("the writer thread does not panic")
        .expect("miniq reads its whole input");
    (
        String::from_utf8(output.stdout).expect("output is UTF-8"),
        output.status.code(),
    )
}

fn error_code(line: &str) -> String {
    let response: serde_json::Value = serde_json::from_str(line).expect("a response is JSON");
    assert_eq!(response["ok"], serde_json::json!(false));
    response["error"]["code"]
        .as_str()
        .expect("an error has a code")
        .to_owned()
}

#[test]
fn every_line_gets_one_response_in_order() {
    let (stdout, status) = run_miniq(b"{\"Command\": \"Tick\", \"Seconds\": 2}\n\nnot json\n{\"Command\": \"Tick\", \"Seconds\": 3}");
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 4);
    assert_eq!(lines[0], r#"{"ok":true,"Now":2}"#);
    assert_eq!(error_code(lines[1]), "InvalidRequest");
    assert_eq!(error_code(lines[2]), "InvalidRequest");
    assert_eq!(lines[3], r#"{"ok":true,"Now":5}"#);
    assert!(stdout.ends_with('\n'));
    assert_eq!(status, Some(0));
}

#[test]
fn invalid_utf8_line_is_rejected_and_the_next_line_still_runs() {
    let (stdout, status) =
        run_miniq(b"{\"Command\": \"\xff\"}\n{\"Command\": \"Tick\", \"Seconds\": 1}\n");
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 2);
    assert_eq!(error_code(lines[0]), "InvalidRequest");
    assert_eq!(lines[1], r#"{"ok":true,"Now":1}"#);
    assert_eq!(status, Some(0));
}

#[test]
fn empty_input_produces_no_output() {
    assert_eq!(run_miniq(b""), (String::new(), Some(0)));
}

#[test]
fn filling_a_queue_to_its_limit_is_fast() {
    let mut input = String::from("{\"Command\": \"CreateQueue\", \"QueueName\": \"q\"}\n");
    for _ in 0..100_001 {
        input.push_str("{\"Command\": \"SendMessage\", \"QueueName\": \"q\", \"Body\": \"x\"}\n");
    }
    let (stdout, _) = run_miniq(input.as_bytes());
    let last = stdout.lines().last().expect("responses exist");
    assert_eq!(error_code(last), "LimitExceeded");
    assert_eq!(stdout.lines().count(), 100_002);
}
