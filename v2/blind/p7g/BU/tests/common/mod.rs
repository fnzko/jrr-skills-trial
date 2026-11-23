use std::io::Cursor;

use serde_json::Value;

/// Feeds raw input to miniq and returns one parsed response per output line.
pub fn run_raw(input: &[u8]) -> Vec<Value> {
    let mut output = Vec::new();
    miniq::run(Cursor::new(input), &mut output).expect("in-memory io does not fail");
    std::str::from_utf8(&output)
        .expect("responses are UTF-8")
        .lines()
        .map(|line| serde_json::from_str(line).expect("every response line is JSON"))
        .collect()
}

/// Runs one command per element and returns the responses.
pub fn run(commands: &[&str]) -> Vec<Value> {
    let input: String = commands.iter().map(|c| format!("{c}\n")).collect();
    run_raw(input.as_bytes())
}

pub fn error_code(response: &Value) -> &str {
    assert_eq!(response["ok"], false, "expected a failure: {response}");
    response["error"]["code"]
        .as_str()
        .expect("error code is a string")
}
