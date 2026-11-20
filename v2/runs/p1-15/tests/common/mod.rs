//! Runs scripted conversations against the real `serve` loop.

#![allow(
    dead_code,
    reason = "each test binary uses a different part of this module"
)]

use serde_json::{Value, json};

pub fn ok() -> Value {
    json!({ "ok": true })
}

/// The expectation for a failure: only the code is compared, the message text is free.
pub fn fail(code: &str) -> Value {
    json!({ "ok": false, "error": code })
}

pub fn run_lines(input: &str) -> Vec<Value> {
    let mut output = Vec::new();
    miniq::serve(input.as_bytes(), &mut output).unwrap();
    String::from_utf8(output)
        .unwrap()
        .lines()
        .map(|line| {
            let mut response: Value = serde_json::from_str(line).unwrap();
            if let Some(code) = response.pointer("/error/code").cloned() {
                response["error"] = code;
            }
            response
        })
        .collect()
}

/// Sends each command and checks its response against the expectation next to it.
#[track_caller]
pub fn script(steps: &[(&str, Value)]) {
    let input: String = steps
        .iter()
        .map(|(command, _)| format!("{command}\n"))
        .collect();
    let responses = run_lines(&input);
    assert_eq!(responses.len(), steps.len());
    for ((command, expected), actual) in steps.iter().zip(&responses) {
        assert_eq!(actual, expected, "response to {command}");
    }
}
