use serde_json::Value;

/// Feeds `input` to miniq and returns one parsed response per output line.
pub fn session(input: &str) -> Vec<Value> {
    let mut output = Vec::new();
    miniq::run(input.as_bytes(), &mut output).expect("writing to a Vec cannot fail");
    String::from_utf8(output)
        .expect("responses are UTF-8")
        .lines()
        .map(|line| serde_json::from_str(line).expect("every response line is JSON"))
        .collect()
}

/// Runs the commands as lines and returns the responses.
pub fn run(commands: &[&str]) -> Vec<Value> {
    let mut input = commands.join("\n");
    input.push('\n');
    session(&input)
}

/// The `error.code` of a failure response, or `None` for a success.
pub fn error_code(response: &Value) -> Option<&str> {
    response["error"]["code"].as_str()
}

/// Runs each command value as one line and returns the responses.
pub fn exec(commands: &[Value]) -> Vec<Value> {
    let lines: Vec<String> = commands.iter().map(Value::to_string).collect();
    let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
    run(&refs)
}
