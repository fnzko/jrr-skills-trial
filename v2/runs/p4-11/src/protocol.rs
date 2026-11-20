//! The line protocol: one command per input line, one response line per
//! command, in input order.

use std::io::{self, BufRead, Write};

use serde_json::{json, Map, Value};

use crate::broker::Broker;
use crate::command::Command;
use crate::error::Error;

/// Reads commands from `input` until its end and writes one response line for
/// each to `output`. Each response is flushed before the next line is read.
///
/// Only I/O failures are returned. A bad command gets a failure response and
/// the loop goes on.
pub fn serve<R: BufRead, W: Write>(mut input: R, mut output: W) -> io::Result<()> {
    let mut broker = Broker::new();
    let mut line = Vec::new();
    loop {
        line.clear();
        if input.read_until(b'\n', &mut line)? == 0 {
            return Ok(());
        }
        if line.last() == Some(&b'\n') {
            line.pop();
        }
        let response = respond(&mut broker, &line);
        writeln!(output, "{response}")?;
        output.flush()?;
    }
}

/// The response to one line of input.
pub fn respond(broker: &mut Broker, line: &[u8]) -> Value {
    match Command::parse_line(line).and_then(|command| broker.execute(command)) {
        Ok(members) => {
            let mut response = Map::new();
            response.insert("ok".into(), Value::Bool(true));
            response.extend(members);
            Value::Object(response)
        }
        Err(error) => failure(&error),
    }
}

fn failure(error: &Error) -> Value {
    json!({
        "ok": false,
        "error": {
            "code": error.code().as_str(),
            "message": error.message(),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(input: &[u8]) -> Vec<String> {
        let mut output = Vec::new();
        serve(input, &mut output).unwrap();
        String::from_utf8(output)
            .unwrap()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    fn code_of(response: &str) -> String {
        let value: Value = serde_json::from_str(response).unwrap();
        value["error"]["code"].as_str().unwrap().to_owned()
    }

    #[test]
    fn each_line_gets_one_response_in_order() {
        let responses = run(
            b"{\"Command\": \"Tick\", \"Seconds\": 5}\n{\"Command\": \"Tick\", \"Seconds\": 2}\n",
        );
        assert_eq!(
            responses,
            [r#"{"Now":5,"ok":true}"#, r#"{"Now":7,"ok":true}"#]
        );
    }

    #[test]
    fn a_blank_line_is_a_command_with_an_invalid_request_response() {
        let responses = run(b"\n");
        assert_eq!(responses.len(), 1);
        assert_eq!(code_of(&responses[0]), "InvalidRequest");
    }

    #[test]
    fn a_final_line_without_a_newline_is_still_a_command() {
        let responses = run(b"{\"Command\": \"Tick\", \"Seconds\": 1}");
        assert_eq!(responses, [r#"{"Now":1,"ok":true}"#]);
    }

    #[test]
    fn a_bad_line_does_not_stop_the_loop() {
        let responses = run(b"not json\n{\"Command\": \"Tick\", \"Seconds\": 1}\n");
        assert_eq!(responses.len(), 2);
        assert_eq!(code_of(&responses[0]), "InvalidRequest");
        assert_eq!(responses[1], r#"{"Now":1,"ok":true}"#);
    }

    #[test]
    fn a_failure_response_has_exactly_the_error_members() {
        let responses = run(b"{\"Command\": \"PurgeQueue\", \"QueueName\": \"nope\"}\n");
        let value: Value = serde_json::from_str(&responses[0]).unwrap();
        assert_eq!(value["ok"], false);
        assert_eq!(value["error"]["code"], "QueueDoesNotExist");
        assert_eq!(value.as_object().unwrap().len(), 2);
        assert_eq!(value["error"].as_object().unwrap().len(), 2);
    }

    #[test]
    fn the_session_state_lasts_for_the_whole_input() {
        let responses = run(b"{\"Command\": \"CreateQueue\", \"QueueName\": \"q\"}\n{\"Command\": \"ListQueues\"}\n");
        assert_eq!(responses[0], r#"{"ok":true}"#);
        assert_eq!(responses[1], r#"{"QueueNames":["q"],"ok":true}"#);
    }

    #[test]
    fn invalid_utf8_is_an_invalid_request() {
        let responses = run(b"\xff\n");
        assert_eq!(code_of(&responses[0]), "InvalidRequest");
    }
}
