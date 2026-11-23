//! Drives miniq through its public entry point, the way the binary does.

use serde_json::{Value, json};

/// Runs raw input through miniq and returns one parsed response per output line.
pub fn run_raw(input: &[u8]) -> Vec<Value> {
    let mut output = Vec::new();
    miniq::run(input, &mut output).expect("reading from a slice and writing to a Vec cannot fail");
    let output = String::from_utf8(output).expect("responses are UTF-8");
    assert!(
        output.is_empty() || output.ends_with('\n'),
        "every response ends in a newline"
    );
    output
        .lines()
        .map(|line| serde_json::from_str(line).expect("every response line is one JSON value"))
        .collect()
}

/// Sends each command on its own line and returns the responses in order.
pub fn exchange(commands: &[Value]) -> Vec<Value> {
    let input: String = commands
        .iter()
        .map(|command| format!("{command}\n"))
        .collect();
    let responses = run_raw(input.as_bytes());
    assert_eq!(responses.len(), commands.len(), "one response per command");
    responses
}

pub fn ok() -> Value {
    json!({"ok": true})
}

/// The error code of a failure response, or `null` for a success.
pub fn code(response: &Value) -> &Value {
    &response["error"]["code"]
}
