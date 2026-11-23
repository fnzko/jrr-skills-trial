//! miniq: a small in-memory message queue driven by JSON lines on stdin.

mod attributes;
mod broker;
mod error;
mod handler;
mod params;
mod queue;
mod types;

use std::io::{self, BufRead, Write};

pub use broker::Broker;
pub use error::{Error, ErrorCode};

/// Serves commands from `input` until it ends, writing and flushing one
/// response line per input line.
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
        let response = handler::respond(&mut broker, &line);
        serde_json::to_writer(&mut output, &response)?;
        output.write_all(b"\n")?;
        output.flush()?;
    }
}
