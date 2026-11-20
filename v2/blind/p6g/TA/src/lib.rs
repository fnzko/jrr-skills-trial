//! miniq: a small in-memory message queue driven by JSON commands on virtual time.

use std::io::{self, BufRead, Write};

pub mod attributes;
pub mod broker;
pub mod error;
pub mod queue;
pub mod request;
pub mod response;
pub mod types;

pub use broker::Broker;

/// Answers every line of `input` with one response line on `output`, flushing after each.
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
        let response = broker.handle_line(&line);
        output.write_all(response.as_bytes())?;
        output.write_all(b"\n")?;
        output.flush()?;
    }
}
