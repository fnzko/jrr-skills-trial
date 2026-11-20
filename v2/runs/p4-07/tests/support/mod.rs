//! Helpers that run scripts through the line protocol for the integration tests.

use miniq::protocol::serve;
use serde_json::{json, Value};

/// Serves `lines` as one input, each line one command, and returns the responses in order.
pub fn replies(lines: &[&str]) -> Vec<Value> {
    let mut input = String::new();
    for line in lines {
        input.push_str(line);
        input.push('\n');
    }
    replies_for_bytes(input.as_bytes())
}

/// Serves raw input bytes and returns the responses in order. Each response is normalized by
/// `normalize`.
pub fn replies_for_bytes(input: &[u8]) -> Vec<Value> {
    let mut output = Vec::new();
    serve(input, &mut output).expect("serving in-memory input does not fail");
    let text = String::from_utf8(output).expect("responses are UTF-8");
    text.lines()
        .map(|line| {
            let reply: Value = serde_json::from_str(line).expect("each response line is JSON");
            normalize(reply)
        })
        .collect()
}

/// A failure is reduced to its code, because the text of a message is free. The test still
/// requires the message member to be a string.
fn normalize(reply: Value) -> Value {
    if reply["ok"] == json!(true) {
        return reply;
    }
    assert!(
        reply["error"]["message"].is_string(),
        "failure carries a message: {reply}"
    );
    json!({"ok": false, "code": reply["error"]["code"]})
}

/// The response of a command that was refused with `code`.
pub fn refused(code: &str) -> Value {
    json!({"ok": false, "code": code})
}
