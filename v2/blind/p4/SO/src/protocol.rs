//! Section 1 of the specification: reading command lines and writing one
//! response line per command.
//!
//! This module only deals with the wire format. What a command does is the
//! business of [`crate::Broker`].

use std::io::{self, BufRead, Write};

use serde_json::{Map, Value};

use crate::{Broker, Error, ErrorCode};

/// A parsed command line: the `Command` name and the remaining members.
#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    pub command: String,
    pub params: Map<String, Value>,
}

/// Parses one line of input into a [`Request`].
///
/// Anything that is not valid UTF-8, not a single JSON object, or an object
/// without a string `Command` member is an `InvalidRequest`. Whether the
/// command name is known is checked later, by the broker.
pub fn parse_request(line: &[u8]) -> Result<Request, Error> {
    let text = std::str::from_utf8(line)
        .map_err(|_| Error::new(ErrorCode::InvalidRequest, "line is not valid UTF-8"))?;
    let value: Value = serde_json::from_str(text)
        .map_err(|e| Error::new(ErrorCode::InvalidRequest, format!("line is not JSON: {e}")))?;
    let Value::Object(mut params) = value else {
        return Err(Error::new(
            ErrorCode::InvalidRequest,
            "line is not a JSON object",
        ));
    };
    match params.remove("Command") {
        Some(Value::String(command)) => Ok(Request { command, params }),
        _ => Err(Error::new(
            ErrorCode::InvalidRequest,
            "object has no string member Command",
        )),
    }
}

/// Builds a success response: `ok` plus the result members of the command.
pub fn ok_response(members: Map<String, Value>) -> Value {
    let mut response = members;
    response.insert("ok".to_owned(), Value::Bool(true));
    Value::Object(response)
}

/// Builds the failure response for `error`.
pub fn error_response(error: &Error) -> Value {
    let mut detail = Map::new();
    detail.insert("code".to_owned(), Value::from(error.code().as_str()));
    detail.insert("message".to_owned(), Value::from(error.message()));

    let mut response = Map::new();
    response.insert("ok".to_owned(), Value::Bool(false));
    response.insert("error".to_owned(), Value::Object(detail));
    Value::Object(response)
}

/// Runs the protocol loop: each line of `input` is one command, and its
/// response is written to `output` and flushed before the next line is read.
///
/// Returns when `input` ends. Only I/O failures are returned as errors; a bad
/// command is an ordinary error response.
pub fn serve<R: BufRead, W: Write>(
    mut input: R,
    mut output: W,
    broker: &mut Broker,
) -> io::Result<()> {
    let mut line = Vec::new();
    loop {
        line.clear();
        if input.read_until(b'\n', &mut line)? == 0 {
            return Ok(());
        }
        if line.last() == Some(&b'\n') {
            line.pop();
        }
        let response = broker.handle_line(&line);
        let bytes = serde_json::to_vec(&response).map_err(io::Error::other)?;
        output.write_all(&bytes)?;
        output.write_all(b"\n")?;
        output.flush()?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn invalid(line: &[u8]) -> Error {
        parse_request(line).expect_err("line should be rejected")
    }

    #[test]
    fn parses_command_and_leaves_out_the_command_member() {
        let request = parse_request(br#"{"Command": "PurgeQueue", "QueueName": "jobs"}"#).unwrap();
        assert_eq!(request.command, "PurgeQueue");
        assert_eq!(request.params.len(), 1);
        assert_eq!(request.params["QueueName"], "jobs");
    }

    #[test]
    fn rejects_lines_that_are_not_utf8() {
        assert_eq!(
            invalid(b"{\"Command\": \"\xff\"}").code(),
            ErrorCode::InvalidRequest
        );
    }

    #[test]
    fn rejects_blank_and_malformed_lines() {
        assert_eq!(invalid(b"").code(), ErrorCode::InvalidRequest);
        assert_eq!(invalid(b"{\"Command\": ").code(), ErrorCode::InvalidRequest);
        assert_eq!(invalid(b"{} trailing").code(), ErrorCode::InvalidRequest);
    }

    #[test]
    fn rejects_non_objects() {
        assert_eq!(invalid(b"[1, 2]").code(), ErrorCode::InvalidRequest);
        assert_eq!(invalid(b"\"Tick\"").code(), ErrorCode::InvalidRequest);
    }

    #[test]
    fn rejects_objects_without_a_string_command() {
        assert_eq!(invalid(b"{}").code(), ErrorCode::InvalidRequest);
        assert_eq!(
            invalid(br#"{"Command": null}"#).code(),
            ErrorCode::InvalidRequest
        );
        assert_eq!(
            invalid(br#"{"Command": 3}"#).code(),
            ErrorCode::InvalidRequest
        );
    }

    #[test]
    fn error_response_has_the_documented_shape() {
        let response = error_response(&Error::new(ErrorCode::LimitExceeded, "full"));
        assert_eq!(
            response,
            serde_json::json!({
                "ok": false,
                "error": {"code": "LimitExceeded", "message": "full"}
            })
        );
    }

    #[test]
    fn ok_response_adds_ok_to_the_result_members() {
        let mut members = Map::new();
        members.insert("Now".to_owned(), Value::from(7));
        assert_eq!(
            ok_response(members),
            serde_json::json!({"ok": true, "Now": 7})
        );
    }
}
