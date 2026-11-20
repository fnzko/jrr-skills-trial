//! miniq: a small in-memory message queue driven by line-delimited JSON commands.

pub mod attributes;
pub mod broker;
pub mod error;
pub mod queue;
pub mod request;

use std::io::{self, BufRead, Write};

use serde_json::{Value, json};

use crate::broker::Broker;

/// Runs one input line against `broker` and renders the response object.
pub fn respond(broker: &mut Broker, line: &[u8]) -> Value {
    match request::parse(line).and_then(|command| broker.execute(command)) {
        Ok(mut result) => {
            result["ok"] = json!(true);
            result
        }
        Err(e) => json!({
            "ok": false,
            "error": { "code": e.code.to_string(), "message": e.message },
        }),
    }
}

/// Answers every line of `input` with one line of `output`, flushing after each, until the
/// input ends. A final line without a newline counts if it is not empty.
pub fn serve(mut input: impl BufRead, mut output: impl Write) -> io::Result<()> {
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
        let mut response = respond(&mut broker, &line).to_string();
        response.push('\n');
        output.write_all(response.as_bytes())?;
        output.flush()?;
    }
}
