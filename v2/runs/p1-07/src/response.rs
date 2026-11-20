//! From one input line to one output line.

use serde_json::{Value, json};

use crate::broker::Broker;
use crate::command;
use crate::error::Error;

/// Runs one input line (without its terminator) and renders the response line
/// (without a trailing newline).
pub fn respond(broker: &mut Broker, line: &[u8]) -> String {
    let outcome = command::parse(line).and_then(|command| broker.execute(command));
    let response = match outcome {
        Ok(mut members) => {
            members.insert("ok".into(), Value::Bool(true));
            Value::Object(members)
        }
        Err(error) => error_json(&error),
    };
    response.to_string()
}

fn error_json(error: &Error) -> Value {
    json!({
        "ok": false,
        "error": { "code": error.code.as_str(), "message": error.message },
    })
}
