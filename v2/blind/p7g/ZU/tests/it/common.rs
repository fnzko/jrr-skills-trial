//! Runs command lines through the real protocol loop and returns the parsed responses.

use serde_json::Value;

pub fn run(lines: &[&str]) -> Vec<Value> {
    let mut input = lines.join("\n");
    input.push('\n');
    run_raw(input.as_bytes())
}

pub fn run_raw(input: &[u8]) -> Vec<Value> {
    let mut output = Vec::new();
    miniq::server::serve(input, &mut output).expect("in-memory io cannot fail");
    let text = String::from_utf8(output).expect("responses are UTF-8");
    assert!(
        text.is_empty() || text.ends_with('\n'),
        "every response ends with a newline"
    );
    text.lines()
        .map(|line| serde_json::from_str(line).expect("every response line is JSON"))
        .collect()
}

pub fn ok() -> Value {
    serde_json::json!({"ok": true})
}

pub fn err(code: &str) -> Value {
    serde_json::json!({"ok": false, "error": {"code": code}})
}

/// Compares responses ignoring the free-text error message.
pub fn assert_responses(actual: Vec<Value>, expected: Vec<Value>) {
    let strip = |mut response: Value| {
        if let Some(error) = response.get_mut("error").and_then(Value::as_object_mut) {
            error.retain(|key, _| key == "code");
        }
        response
    };
    let actual: Vec<Value> = actual.into_iter().map(strip).collect();
    assert_eq!(actual, expected);
}
