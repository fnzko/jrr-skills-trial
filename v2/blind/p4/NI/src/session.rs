//! The line protocol over a reader and a writer (SPEC section 1).
//!
//! One input line is one command, and it gets exactly one response line. The
//! loop reads a line, answers it, and flushes before reading the next, so an
//! interactive client sees each answer before it sends the following command.

use std::io::{self, BufRead, Write};

use serde_json::Value;

use crate::broker::Broker;
use crate::command::parse_line;
use crate::response::{failure, success};

/// Answers every line of `input` in order, writing one JSON response per line.
///
/// Returns `Ok` at end of input. A final line without a newline still counts
/// unless it is empty. Blank lines are commands too, and draw `InvalidRequest`.
pub fn serve<R: BufRead, W: Write>(
    broker: &mut Broker,
    mut input: R,
    mut output: W,
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

        let response = respond(broker, &line);
        let text = serde_json::to_string(&response).map_err(io::Error::other)?;
        output.write_all(text.as_bytes())?;
        output.write_all(b"\n")?;
        output.flush()?;
    }
}

fn respond(broker: &mut Broker, line: &[u8]) -> Value {
    match parse_line(line).and_then(|command| broker.execute(command)) {
        Ok(result) => success(result),
        Err(error) => failure(&error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Runs `input` through a fresh broker and returns the response lines, parsed.
    fn run(input: &[u8]) -> Vec<Value> {
        let mut broker = Broker::new();
        let mut output = Vec::new();
        serve(&mut broker, input, &mut output).unwrap();
        String::from_utf8(output)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    fn raw(input: &[u8]) -> String {
        let mut broker = Broker::new();
        let mut output = Vec::new();
        serve(&mut broker, input, &mut output).unwrap();
        String::from_utf8(output).unwrap()
    }

    #[test]
    fn empty_input_produces_no_output() {
        assert_eq!(raw(b""), "");
    }

    #[test]
    fn each_line_gets_one_response_in_input_order() {
        let responses =
            run(b"{\"Command\":\"Tick\",\"Seconds\":3}\n{\"Command\":\"Tick\",\"Seconds\":4}\n");
        assert_eq!(
            responses,
            vec![json!({"ok": true, "Now": 3}), json!({"ok": true, "Now": 7})]
        );
    }

    #[test]
    fn a_final_line_without_a_newline_counts() {
        let responses = run(b"{\"Command\":\"Tick\",\"Seconds\":1}");
        assert_eq!(responses, vec![json!({"ok": true, "Now": 1})]);
    }

    #[test]
    fn blank_lines_are_commands_that_are_invalid() {
        let responses = run(b"\n\n");
        assert_eq!(responses.len(), 2);
        for response in responses {
            assert_eq!(response["error"]["code"], json!("InvalidRequest"));
        }
    }

    #[test]
    fn a_line_that_is_not_utf8_is_invalid_and_the_loop_continues() {
        let mut input = vec![0xff, 0xfe, b'\n'];
        input.extend_from_slice(b"{\"Command\":\"Tick\",\"Seconds\":2}\n");
        let responses = run(&input);
        assert_eq!(responses[0]["error"]["code"], json!("InvalidRequest"));
        assert_eq!(responses[1], json!({"ok": true, "Now": 2}));
    }

    #[test]
    fn crlf_line_endings_are_accepted() {
        let responses = run(b"{\"Command\":\"Tick\",\"Seconds\":5}\r\n");
        assert_eq!(responses, vec![json!({"ok": true, "Now": 5})]);
    }

    #[test]
    fn every_line_is_one_compact_object_ending_in_newline() {
        let text = raw(b"{\"Command\":\"ListQueues\"}\n");
        assert!(text.ends_with('\n'));
        assert_eq!(text.matches('\n').count(), 1);
    }

    #[test]
    fn a_failed_line_does_not_stop_later_lines() {
        let responses = run(b"{\"Command\":\"Nope\"}\n{\"Command\":\"ListQueues\"}\n");
        assert_eq!(responses[0]["ok"], json!(false));
        assert_eq!(responses[1], json!({"ok": true, "QueueNames": []}));
    }
}
