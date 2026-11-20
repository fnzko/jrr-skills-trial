//! miniq: a small in-memory message queue driven by JSON lines.

mod error;
mod params;
mod queue;
mod reply;
mod request;
mod service;
mod types;

use std::io::{self, BufRead, Write};

pub use error::{Error, ErrorCode};
pub use service::Service;

/// Answers every line of `input` with one response line on `output`, in
/// order, flushing after each, until the end of input.
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
        let mut response = service.handle_line(&line);
        response.push('\n');
        output.write_all(response.as_bytes())?;
        output.flush()?;
    }
}
