//! miniq: a small in-memory message queue driven by JSON lines.
//!
//! [`run`] implements the line protocol; [`Service`] holds all state and can
//! be driven directly with [`Command`]s.

mod command;
mod error;
mod params;
mod queue;
mod service;

use std::io::{self, BufRead, Write};

use serde_json::{json, Value};

pub use command::Command;
pub use error::{Error, ErrorCode};
pub use service::Service;

/// Reads commands line by line from `input` and writes one response line per
/// command to `output`, flushing after each, until end of input.
pub fn run<R: BufRead, W: Write>(mut input: R, mut output: W) -> io::Result<()> {
    let mut service = Service::new();
    let mut line = Vec::new();
    loop {
        line.clear();
        if input.read_until(b'\n', &mut line)? == 0 {
            return Ok(());
        }
        if line.last() == Some(&b'\n') {
            line.pop();
        }
        let response = handle_line(&mut service, &line);
        output.write_all(response.as_bytes())?;
        output.write_all(b"\n")?;
        output.flush()?;
    }
}

/// Executes one request line and renders the response line (without `\n`).
pub fn handle_line(service: &mut Service, line: &[u8]) -> String {
    let outcome = command::parse(line).and_then(|command| service.execute(command));
    let response = match outcome {
        Ok(Value::Object(mut members)) => {
            members.insert("ok".into(), Value::Bool(true));
            Value::Object(members)
        }
        Ok(_) => unreachable!("commands return JSON objects"),
        Err(error) => json!({
            "ok": false,
            "error": { "code": error.code.as_str(), "message": error.message },
        }),
    };
    response.to_string()
}
