//! Helpers shared by the integration tests: send a command, get the parsed
//! response back.

#![allow(dead_code)]

use miniq::Broker;
use serde_json::Value;

/// Sends one command line to `broker` and returns the response object.
pub fn send(broker: &mut Broker, line: &str) -> Value {
    broker.handle_line(line.as_bytes())
}

/// Sends a command and asserts that it succeeded; returns the response.
pub fn ok(broker: &mut Broker, line: &str) -> Value {
    let response = send(broker, line);
    assert_eq!(response["ok"], true, "{line} failed: {response}");
    response
}

/// Sends a command and asserts that it failed with `code`.
pub fn fail(broker: &mut Broker, line: &str, code: &str) {
    let response = send(broker, line);
    assert_eq!(response["ok"], false, "{line} should fail: {response}");
    assert_eq!(response["error"]["code"], code, "{line}: {response}");
}
