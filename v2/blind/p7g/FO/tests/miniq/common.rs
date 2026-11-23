use serde_json::{Value, json};

/// Feeds the commands to miniq as lines and returns the parsed responses.
pub fn session(commands: &[Value]) -> Vec<Value> {
    let input: String = commands.iter().map(|c| format!("{c}\n")).collect();
    run_text(input.as_bytes())
}

pub fn run_text(input: &[u8]) -> Vec<Value> {
    let mut output = Vec::new();
    miniq::run(input, &mut output).expect("reading from a slice and writing to a Vec cannot fail");
    let text = String::from_utf8(output).expect("responses are UTF-8");
    assert!(
        text.is_empty() || text.ends_with('\n'),
        "every response ends in a newline"
    );
    text.lines()
        .map(|line| serde_json::from_str(line).expect("each response is one JSON object"))
        .collect()
}

pub fn ok() -> Value {
    json!({"ok": true})
}

/// Asserts a failure response with the given code; the message text is unspecified.
pub fn assert_error(response: &Value, code: &str) {
    assert!(
        response["ok"] == json!(false) && response["error"]["code"] == json!(code),
        "expected error {code}, got {response}"
    );
}

pub fn create(name: &str) -> Value {
    json!({"Command": "CreateQueue", "QueueName": name})
}

pub fn create_with(name: &str, attributes: Value) -> Value {
    json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attributes})
}

pub fn send(queue: &str, body: &str) -> Value {
    json!({"Command": "SendMessage", "QueueName": queue, "Body": body})
}

pub fn receive(queue: &str) -> Value {
    json!({"Command": "ReceiveMessage", "QueueName": queue})
}

pub fn receive_max(queue: &str, max: u32) -> Value {
    json!({"Command": "ReceiveMessage", "QueueName": queue, "MaxMessages": max})
}

pub fn tick(seconds: u64) -> Value {
    json!({"Command": "Tick", "Seconds": seconds})
}

pub fn attributes(queue: &str) -> Value {
    json!({"Command": "GetQueueAttributes", "QueueName": queue})
}

/// The bodies of a `ReceiveMessage` response, in delivery order.
pub fn bodies(response: &Value) -> Vec<&str> {
    response["Messages"]
        .as_array()
        .expect("a successful receive has Messages")
        .iter()
        .map(|m| m["Body"].as_str().expect("Body is a string"))
        .collect()
}
