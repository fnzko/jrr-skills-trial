//! miniq, a small in-memory message queue driven by one JSON command per line.

mod error;
mod queue;
mod request;
mod response;
mod service;

use std::io::{self, BufRead, Write};

pub use service::Service;

/// Answers every line of `input` with one line on `output`, flushing after each.
///
/// A final line without `\n` counts when it is not empty.
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
        let response = service.handle_line(&line);
        output.write_all(response.as_bytes())?;
        output.write_all(b"\n")?;
        output.flush()?;
    }
}
