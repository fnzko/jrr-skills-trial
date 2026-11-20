//! The line protocol: one response per input line, flushed before the next
//! line is read.

use std::io::{self, BufRead, Write};

use crate::command::Command;
use crate::output::{Failure, Success};
use crate::service::Service;

/// Serves commands from `input` until it ends. Only I/O failures are errors;
/// a bad command gets an error response and the next line is read.
pub fn serve(mut input: impl BufRead, mut output: impl Write) -> io::Result<()> {
    let mut service = Service::new();
    let mut line = Vec::new();
    let mut response = Vec::new();
    loop {
        line.clear();
        if input.read_until(b'\n', &mut line)? == 0 {
            return Ok(());
        }
        if line.last() == Some(&b'\n') {
            line.pop();
        }
        response.clear();
        match Command::parse(&line).and_then(|command| service.execute(command)) {
            Ok(result) => serde_json::to_writer(
                &mut response,
                &Success {
                    ok: true,
                    output: &result,
                },
            )?,
            Err(error) => serde_json::to_writer(
                &mut response,
                &Failure {
                    ok: false,
                    error: &error,
                },
            )?,
        }
        response.push(b'\n');
        output.write_all(&response)?;
        output.flush()?;
    }
}
