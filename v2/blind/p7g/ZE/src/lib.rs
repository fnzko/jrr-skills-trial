//! miniq: a small in-memory message queue driven by JSON command lines.

use std::io::{self, BufRead, Write};

pub mod attributes;
pub mod error;
pub mod json;
pub mod queue;
pub mod reply;
pub mod request;
pub mod service;
pub mod types;

pub use service::Miniq;

/// Answers every line of `input` with one response line on `output`, flushing
/// after each so a peer can drive the service interactively.
pub fn run(mut input: impl BufRead, mut output: impl Write) -> io::Result<()> {
    let mut miniq = Miniq::new();
    let mut line = Vec::new();
    while input.read_until(b'\n', &mut line)? > 0 {
        if line.last() == Some(&b'\n') {
            line.pop();
        }
        let response = miniq.handle_line(&line);
        output.write_all(response.as_bytes())?;
        output.write_all(b"\n")?;
        output.flush()?;
        line.clear();
    }
    Ok(())
}
