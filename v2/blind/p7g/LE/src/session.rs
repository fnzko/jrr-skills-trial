//! The line protocol: one response per input line, flushed immediately.

use std::io::{self, BufRead, Write};

use crate::broker::Broker;
use crate::command;
use crate::reply;

/// Serves commands from `input` until it ends.
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
        let outcome = command::parse(&line).and_then(|command| broker.execute(command));
        let response = reply::render(&outcome).map_err(io::Error::other)?;
        writeln!(output, "{response}")?;
        output.flush()?;
    }
}
