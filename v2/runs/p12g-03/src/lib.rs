//! miniq: a small in-memory message queue driven by JSON command lines.

mod attributes;
mod command;
mod error;
mod handles;
mod params;
mod queue;
mod response;
mod service;
mod types;

use std::io::{self, BufRead, Write};

pub use service::Service;

/// Answers every line of `input` with one response line on `output`, flushing
/// after each, until the input ends.
pub fn run(mut input: impl BufRead, mut output: impl Write) -> io::Result<()> {
    let mut service = Service::default();
    let mut line = Vec::new();
    loop {
        line.clear();
        if input.read_until(b'\n', &mut line)? == 0 {
            return Ok(());
        }
        if line.last() == Some(&b'\n') {
            line.pop();
        }
        let response = service.handle_line(&line);
        serde_json::to_writer(&mut output, &response)?;
        output.write_all(b"\n")?;
        output.flush()?;
    }
}
