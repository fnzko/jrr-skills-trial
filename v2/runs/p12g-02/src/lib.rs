//! miniq: a small in-memory message queue driven by one JSON command per input line.

mod broker;
mod error;
mod queue;
mod request;
mod response;
mod types;

use std::io::{self, BufRead, Write};

use broker::Broker;

/// Answers every line of `input` with one response line on `output`, flushing after each,
/// until the input ends.
pub fn run(mut input: impl BufRead, mut output: impl Write) -> io::Result<()> {
    let mut broker = Broker::new();
    let mut line = Vec::new();
    loop {
        line.clear();
        if input.read_until(b'\n', &mut line)? == 0 {
            return Ok(());
        }
        if line.ends_with(b"\n") {
            line.pop();
        }
        let result = request::parse_line(&line).and_then(|request| broker.execute(request));
        response::write_response(&mut output, &result)?;
    }
}
