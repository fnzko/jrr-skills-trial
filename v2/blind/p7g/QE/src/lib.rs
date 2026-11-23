//! miniq: a small in-memory message queue driven by line-delimited JSON commands.

mod broker;
mod error;
mod message_commands;
mod params;
mod queue;
mod queue_commands;

use std::io::{self, BufRead, Write};

pub use broker::Broker;
pub use error::{Error, ErrorCode};

/// Serves commands from `input` until it ends, writing one flushed response line
/// per command to `output`.
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
        let response = broker.handle_line(&line);
        serde_json::to_writer(&mut output, &response)?;
        output.write_all(b"\n")?;
        output.flush()?;
    }
}
