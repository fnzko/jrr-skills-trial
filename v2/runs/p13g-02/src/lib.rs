//! miniq: an in-memory message queue driven by JSON commands, one per line.

pub mod broker;
pub mod command;
pub mod error;
pub mod queue;
pub mod types;

use std::io::{self, BufRead, Write};

use broker::Broker;

/// Answers every line of `input` with one response line on `output`, flushing
/// after each so a caller can read a response before sending the next command.
pub fn run(mut input: impl BufRead, mut output: impl Write) -> io::Result<()> {
    let mut broker = Broker::default();
    let mut line = Vec::new();
    loop {
        line.clear();
        if input.read_until(b'\n', &mut line)? == 0 {
            return Ok(());
        }
        if line.last() == Some(&b'\n') {
            line.pop();
        }
        let response = broker.respond(&line);
        output.write_all(response.as_bytes())?;
        output.write_all(b"\n")?;
        output.flush()?;
    }
}
