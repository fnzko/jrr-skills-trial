//! miniq: a small in-memory message queue with virtual time.
//!
//! [`run`] reads commands from a reader and writes one response per line to
//! a writer; the `miniq` binary wires it to stdin and stdout.

pub mod broker;
pub mod config;
pub mod error;
pub mod ids;
pub mod queue;
pub mod request;

use std::io::{self, BufRead, Write};

/// Answers every line of `input` with one response line on `output`,
/// flushing after each, until the input ends.
pub fn run(mut input: impl BufRead, mut output: impl Write) -> io::Result<()> {
    let mut broker = broker::Broker::default();
    let mut line = Vec::new();
    loop {
        line.clear();
        if input.read_until(b'\n', &mut line)? == 0 {
            return Ok(());
        }
        if line.last() == Some(&b'\n') {
            line.pop();
        }
        let mut response = broker.handle_line(&line).to_string();
        response.push('\n');
        output.write_all(response.as_bytes())?;
        output.flush()?;
    }
}
