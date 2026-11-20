//! miniq: a small in-memory message queue driven by JSON commands, one per line.

mod broker;
mod error;
mod queue;
mod receipt;
mod request;
mod response;

use std::io::{self, BufRead, Write};

pub use broker::Broker;
pub use error::{Error, ErrorCode};

/// Answers every line of `input` with one response line on `output`.
///
/// Each response is flushed before the next line is read, so a peer that waits for an
/// answer before sending more never stalls. A final line without a newline still counts.
pub fn run(mut input: impl BufRead, mut output: impl Write) -> io::Result<()> {
    let mut broker = Broker::new();
    let mut line = Vec::new();
    loop {
        line.clear();
        if input.read_until(b'\n', &mut line)? == 0 {
            return Ok(());
        }
        if line.last() == Some(&b'\n') {
            line.pop();
        }
        let mut response = broker.respond(&line);
        response.push('\n');
        output.write_all(response.as_bytes())?;
        output.flush()?;
    }
}
