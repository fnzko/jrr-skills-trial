#![allow(dead_code)]

use serde_json::Value;

/// Feeds `input` to miniq and returns each response line parsed as JSON.
pub fn run_bytes(input: &[u8]) -> Vec<Value> {
    let mut output = Vec::new();
    miniq::serve(input, &mut output).expect("serve should not fail on in-memory I/O");
    let text = String::from_utf8(output).expect("responses are UTF-8");
    assert!(
        text.is_empty() || text.ends_with('\n'),
        "every response line ends in a newline: {text:?}"
    );
    text.lines()
        .map(|line| serde_json::from_str(line).expect("each response line is one JSON object"))
        .collect()
}

/// Feeds newline-separated commands to miniq and returns the responses.
pub fn run(input: &str) -> Vec<Value> {
    run_bytes(input.as_bytes())
}

/// Runs `lines` as one session and returns the responses.
pub fn session(lines: &[&str]) -> Vec<Value> {
    run(&lines.join("\n"))
}

/// Runs `lines` as one session and returns the response to the last one.
pub fn last(lines: &[&str]) -> Value {
    session(lines).pop().expect("at least one response")
}

pub fn assert_ok(response: &Value) {
    assert_eq!(response["ok"], true, "expected success, got {response}");
}

pub fn assert_error(response: &Value, code: &str) {
    assert_eq!(response["ok"], false, "expected failure, got {response}");
    assert_eq!(response["error"]["code"], code, "wrong error in {response}");
}
