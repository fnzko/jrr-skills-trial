//! miniq: a small in-memory message queue driven by JSON lines.

mod attributes;
mod command;
mod error;
mod ids;
mod params;
mod queue;
mod reply;
mod service;
mod time;

use std::io::{self, BufRead, Write};

pub use error::{Error, ErrorCode};
pub use reply::{Reply, Response};
pub use service::Service;

/// Answers every line of `input` with one response line on `output`.
///
/// Each response is flushed before the next line is read. A final line
/// without a newline counts when it is not empty.
pub fn run<R: BufRead, W: Write>(mut input: R, mut output: W) -> io::Result<()> {
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
