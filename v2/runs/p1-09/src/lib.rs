//! miniq: a small in-memory message queue driven by JSON commands, one per line.

mod attributes;
mod broker;
mod error;
mod queue;
mod receipts;
mod request;

use std::io::{self, BufRead, Write};

pub use broker::Broker;
pub use error::{Error, ErrorCode};

/// Answers every line of `input` with one line on `output`, flushing after each,
/// until the input ends.
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
        let mut response = broker.handle_line(&line);
        response.push('\n');
        output.write_all(response.as_bytes())?;
        output.flush()?;
    }
}
