//! Helpers shared by the integration tests.

#![allow(dead_code, reason = "each test binary uses a subset of the helpers")]

use std::io::Cursor;

use serde_json::Value;

/// Feeds `input` to the service and returns each response line parsed.
pub fn run_raw(input: &[u8]) -> Vec<Value> {
    let mut output = Vec::new();
    miniq::run(Cursor::new(input), &mut output).unwrap();
    let text = String::from_utf8(output).unwrap();
    assert!(text.is_empty() || text.ends_with('\n'), "output {text:?}");
    text.lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

/// Runs one command per line and returns the responses.
pub fn run(lines: &[&str]) -> Vec<Value> {
    let mut input = lines.join("\n").into_bytes();
    input.push(b'\n');
    run_raw(&input)
}

/// The error code of a failure response; panics on success.
pub fn error_code(response: &Value) -> &str {
    assert_eq!(response["ok"], false, "{response}");
    response["error"]["code"].as_str().unwrap()
}

/// Runs one command per element and returns the responses.
pub fn session(commands: &[Value]) -> Vec<Value> {
    let lines: Vec<String> = commands.iter().map(Value::to_string).collect();
    let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
    run(&lines)
}

/// The expected response of a command that succeeds with no result members.
pub fn ok() -> Value {
    serde_json::json!({"ok": true})
}

/// `"ok"` for a success and the error code for a failure, one per response.
pub fn outcomes(responses: &[Value]) -> Vec<&str> {
    responses
        .iter()
        .map(|response| match response["ok"].as_bool() {
            Some(true) => "ok",
            _ => error_code(response),
        })
        .collect()
}
