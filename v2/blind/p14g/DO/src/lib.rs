//! miniq: a small in-memory message queue driven by one JSON command per line.

mod broker;
mod error;
mod queue;
mod request;
mod response;
mod store;
mod types;

use std::io::{self, BufRead, Write};

use broker::Broker;
use request::Request;

/// Answers every line of `input` with one response line on `output`, flushing after each, until
/// the end of input.
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
        let outcome = Request::parse(&line).and_then(|request| broker.execute(request));
        let response = response::render(&outcome)?;
        output.write_all(response.as_bytes())?;
        output.write_all(b"\n")?;
        output.flush()?;
    }
}
