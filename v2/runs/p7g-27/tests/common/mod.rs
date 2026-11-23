//! Drives the miniq loop in-process and compares whole responses.

use serde_json::Value;

/// Runs `lines` through miniq and returns one parsed response per line.
pub fn session(lines: &[&str]) -> Vec<Value> {
    let input = lines.join("\n") + "\n";
    let mut output = Vec::new();
    miniq::run(input.as_bytes(), &mut output).expect("in-memory io cannot fail");
    String::from_utf8(output)
        .expect("responses are UTF-8")
        .lines()
        .map(|line| serde_json::from_str(line).expect("each response is one JSON object"))
        .collect()
}

pub fn error_code(response: &Value) -> &str {
    assert_eq!(response["ok"], false, "expected a failure: {response}");
    response["error"]["code"]
        .as_str()
        .expect("error has a code")
}

/// Runs commands given as JSON values.
pub fn run(commands: &[Value]) -> Vec<Value> {
    let lines: Vec<String> = commands.iter().map(Value::to_string).collect();
    let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
    session(&refs)
}
