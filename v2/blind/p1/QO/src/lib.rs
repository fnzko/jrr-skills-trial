//! miniq: a small in-memory message queue driven by one JSON command per
//! input line. All state is in the process and time is virtual.

mod broker;
mod deliveries;
mod error;
mod queue;
mod request;
mod response;
mod types;

use std::io::{self, BufRead, Write};

use broker::Broker;
use request::Command;
use response::write_response;

/// Answers every line of `input` with one response line on `output`, in
/// order, flushing after each. Returns at end of input.
///
/// Fails only on I/O errors; protocol errors are answered, not returned.
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
        let result = Command::parse(&line).and_then(|command| broker.execute(command));
        write_response(&mut output, &result)?;
    }
}
