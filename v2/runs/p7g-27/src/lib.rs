//! miniq: a small in-memory message queue driven by JSON lines on stdin.

mod attributes;
mod broker;
mod command;
mod error;
mod handles;
mod params;
mod queue;
mod response;
mod types;

use std::io::{self, BufRead, Write};

use broker::Broker;
use command::Command;

/// Answers every line of `input` with one response line on `output`, flushing after each.
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
        let outcome = Command::parse(&line).and_then(|command| broker.execute(command));
        let text = response::render(&outcome).map_err(io::Error::other)?;
        writeln!(output, "{text}")?;
        output.flush()?;
    }
}
