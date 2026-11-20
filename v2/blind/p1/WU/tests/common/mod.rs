#![allow(dead_code)]

use serde_json::Value;

/// Runs miniq over `input` and returns one parsed response per output line.
pub fn run_bytes(input: &[u8]) -> Vec<Value> {
    let mut output = Vec::new();
    miniq::run(input, &mut output).unwrap();
    let text = String::from_utf8(output).unwrap();
    assert!(text.is_empty() || text.ends_with('\n'), "output {text:?}");
    text.lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

pub fn run(input: &str) -> Vec<Value> {
    run_bytes(input.as_bytes())
}

/// Runs one command per element and returns the responses.
pub fn session(commands: &[&str]) -> Vec<Value> {
    let mut input = commands.join("\n");
    input.push('\n');
    let responses = run(&input);
    assert_eq!(responses.len(), commands.len());
    responses
}

#[track_caller]
pub fn assert_error(response: &Value, code: &str) {
    assert_eq!(response["ok"], false, "{response}");
    assert_eq!(response["error"]["code"], code, "{response}");
    assert!(response["error"]["message"].is_string(), "{response}");
}

/// A miniq instance that tests drive one command at a time.
#[derive(Default)]
pub struct Miniq(miniq::Service);

impl Miniq {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn call(&mut self, command: Value) -> Value {
        self.0.handle(command.to_string().as_bytes())
    }

    /// Calls a command that must succeed and returns its response.
    #[track_caller]
    pub fn ok(&mut self, command: Value) -> Value {
        let response = self.call(command);
        assert_eq!(response["ok"], true, "{response}");
        response
    }

    /// Calls a command that must fail with `code`.
    #[track_caller]
    pub fn fails(&mut self, command: Value, code: &str) {
        let response = self.call(command);
        assert_error(&response, code);
    }
}
