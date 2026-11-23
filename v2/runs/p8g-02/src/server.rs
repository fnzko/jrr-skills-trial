//! The line protocol: one command per input line, one JSON response per command.

use std::io::{self, BufRead, Write};

use crate::broker::Broker;
use crate::request;
use crate::response::{Failure, Success};

/// Reads commands until end of input. Each response is written and flushed before the next
/// line is read, so a peer that waits for answers never deadlocks.
pub fn serve(mut input: impl BufRead, mut output: impl Write) -> io::Result<()> {
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
        match request::parse(&line).and_then(|request| broker.execute(request)) {
            Ok(reply) => serde_json::to_writer(&mut output, &Success::new(&reply))?,
            Err(error) => serde_json::to_writer(&mut output, &Failure::new(&error))?,
        }
        output.write_all(b"\n")?;
        output.flush()?;
    }
}
