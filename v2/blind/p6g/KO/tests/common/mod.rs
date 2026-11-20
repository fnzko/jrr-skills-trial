// Each test binary uses a different subset of these helpers.
#![allow(dead_code)]

use serde_json::Value;

/// Runs `input` through miniq and returns the parsed response lines.
///
/// Panics unless there is exactly one well-formed response line per input line.
#[track_caller]
pub fn run_raw(input: &str) -> Vec<Value> {
    let mut output = Vec::new();
    miniq::run(input.as_bytes(), &mut output).unwrap();
    let text = String::from_utf8(output).unwrap();
    assert!(text.is_empty() || text.ends_with('\n'), "output: {text:?}");
    text.lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

/// Runs one command per element and returns one response per command.
#[track_caller]
pub fn run(commands: &[&str]) -> Vec<Value> {
    let mut input = commands.join("\n");
    input.push('\n');
    let responses = run_raw(&input);
    assert_eq!(responses.len(), commands.len());
    responses
}

pub fn error_code(response: &Value) -> &str {
    assert_eq!(response["ok"], false, "{response}");
    response["error"]["code"].as_str().unwrap()
}
