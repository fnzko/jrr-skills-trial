use miniq::run;
use serde_json::Value;

/// Feeds `commands` (one JSON object per element) to miniq and returns the
/// parsed response lines.
pub fn session(commands: &[&str]) -> Vec<Value> {
    let mut input = commands.join("\n");
    input.push('\n');
    session_bytes(input.as_bytes())
}

pub fn session_bytes(input: &[u8]) -> Vec<Value> {
    let mut out = Vec::new();
    run(input, &mut out).expect("in-memory io cannot fail");
    String::from_utf8(out)
        .expect("responses are UTF-8")
        .lines()
        .map(|line| serde_json::from_str(line).expect("each response is one JSON line"))
        .collect()
}

/// The error code of a failure response, or `None` for a success.
pub fn error_code(response: &Value) -> Option<&str> {
    response["error"]["code"].as_str()
}
