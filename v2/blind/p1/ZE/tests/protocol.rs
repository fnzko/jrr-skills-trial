mod common;

use common::{error_code, run, run_raw};
use serde_json::json;

#[test]
fn every_line_gets_one_response_in_order() {
    let responses = run(&[
        r#"{"Command":"Tick","Seconds":2}"#,
        "",
        r#"{"Command":"Tick","Seconds":3}"#,
    ]);
    assert_eq!(responses.len(), 3);
    assert_eq!(responses[0], json!({"ok": true, "Now": 2}));
    assert_eq!(error_code(&responses[1]), "InvalidRequest");
    assert_eq!(responses[2], json!({"ok": true, "Now": 5}));
}

#[test]
fn a_final_line_without_newline_counts() {
    let responses = run_raw(br#"{"Command":"Tick","Seconds":1}"#);
    assert_eq!(responses, [json!({"ok": true, "Now": 1})]);
}

#[test]
fn no_input_gives_no_output() {
    assert!(run_raw(b"").is_empty());
}

#[test]
fn invalid_utf8_is_an_invalid_request_and_the_next_line_still_runs() {
    let responses = run_raw(b"\xff\xfe\n{\"Command\":\"Tick\",\"Seconds\":1}\n");
    assert_eq!(error_code(&responses[0]), "InvalidRequest");
    assert_eq!(responses[1], json!({"ok": true, "Now": 1}));
}

#[test]
fn failure_responses_carry_a_code_and_a_message() {
    let responses = run(&[r#"{"Command":"Tick"}"#]);
    assert_eq!(responses[0]["ok"], false);
    assert_eq!(responses[0]["error"]["code"], "MissingParameter");
    assert!(responses[0]["error"]["message"].is_string());
}

#[test]
fn the_binary_answers_on_stdout() {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let mut child = Command::new(env!("CARGO_BIN_EXE_miniq"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"{\"Command\":\"Tick\",\"Seconds\":4}")
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.ends_with('\n'));
    let response: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(response, json!({"ok": true, "Now": 4}));
}
