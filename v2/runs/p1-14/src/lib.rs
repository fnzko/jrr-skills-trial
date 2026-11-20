//! miniq: a small in-memory message queue driven by JSON commands, one per line.

mod attributes;
mod error;
mod ids;
mod name;
mod params;
mod queue;
mod request;
mod service;

use std::io::{self, BufRead, Write};

pub use service::Service;

/// Answers every line of `input` with one response line on `output`.
///
/// Each response is flushed before the next line is read. Returns at end of
/// input.
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
        writeln!(output, "{}", service.handle(&line))?;
        output.flush()?;
    }
}
