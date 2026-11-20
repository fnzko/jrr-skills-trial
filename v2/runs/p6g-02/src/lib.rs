//! miniq: a small in-memory message queue driven by JSON lines.

use std::io::{self, BufRead, Write};

pub mod config;
pub mod error;
pub mod ids;
pub mod queue;
pub mod request;
pub mod response;
pub mod service;

use service::Service;

/// Answers every line of `input` with one JSON line on `output`, flushing
/// after each, until the input ends.
pub fn run(mut input: impl BufRead, mut output: impl Write) -> io::Result<()> {
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
        let result = request::parse(&line).and_then(|command| service.execute(command));
        writeln!(output, "{}", response::render(&result))?;
        output.flush()?;
    }
}
