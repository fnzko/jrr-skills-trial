use std::io::Cursor;

use serde_json::{Value, json};

/// Feeds raw bytes to a fresh server and returns every response line, parsed.
pub fn run_bytes(input: &[u8]) -> Vec<Value> {
    let mut output = Vec::new();
    miniq::server::serve(Cursor::new(input), &mut output).expect("in-memory I/O cannot fail");
    let text = String::from_utf8(output).expect("responses are UTF-8");
    assert!(
        text.is_empty() || text.ends_with('\n'),
        "every response ends in a newline"
    );
    text.lines()
        .map(|line| serde_json::from_str(line).expect("every response line is JSON"))
        .collect()
}

pub fn run(commands: &[Value]) -> Vec<Value> {
    let input: String = commands
        .iter()
        .map(|command| format!("{command}\n"))
        .collect();
    let responses = run_bytes(input.as_bytes());
    assert_eq!(responses.len(), commands.len(), "one response per command");
    responses
}

pub fn ok() -> Value {
    json!({"ok": true})
}

/// The error code of a failure response.
pub fn code(response: &Value) -> &str {
    assert_eq!(
        response["ok"],
        json!(false),
        "expected a failure: {response}"
    );
    response["error"]["code"]
        .as_str()
        .expect("error has a code")
}

pub fn create(name: &str, attributes: Value) -> Value {
    json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attributes})
}

pub fn send(queue: &str, body: &str) -> Value {
    json!({"Command": "SendMessage", "QueueName": queue, "Body": body})
}

pub fn receive(queue: &str) -> Value {
    json!({"Command": "ReceiveMessage", "QueueName": queue})
}

pub fn tick(seconds: u32) -> Value {
    json!({"Command": "Tick", "Seconds": seconds})
}

pub fn counts(queue: &str) -> Value {
    json!({"Command": "GetQueueAttributes", "QueueName": queue})
}
