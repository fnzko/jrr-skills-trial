//! miniq: a small in-memory message queue driven by JSON commands on stdin.

mod attributes;
mod error;
mod ids;
mod queue;
mod reply;
mod request;
mod service;

use std::io::{self, BufRead, Write};

/// Answers every line of `input` with one response line on `output`,
/// flushing after each, until the input ends.
pub fn run(mut input: impl BufRead, mut output: impl Write) -> io::Result<()> {
    let mut service = service::Service::new();
    let mut line = Vec::new();
    loop {
        line.clear();
        if input.read_until(b'\n', &mut line)? == 0 {
            return Ok(());
        }
        if line.last() == Some(&b'\n') {
            line.pop();
        }
        let outcome = request::parse_line(&line).and_then(|command| service.execute(command));
        writeln!(output, "{}", reply::render(&outcome))?;
        output.flush()?;
    }
}
