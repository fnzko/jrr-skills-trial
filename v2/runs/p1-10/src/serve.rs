//! The line protocol: one command per input line, one response per command.

use std::io::{self, BufRead, Write};

use crate::broker::Broker;

/// Answers every line of `input` on `output`, flushing after each response, until the input
/// ends. A final line without a newline counts if it is not empty.
pub fn serve(mut input: impl BufRead, mut output: impl Write) -> io::Result<()> {
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
        let response = broker.respond(&line).map_err(io::Error::other)?;
        output.write_all(response.as_bytes())?;
        output.write_all(b"\n")?;
        output.flush()?;
    }
}
