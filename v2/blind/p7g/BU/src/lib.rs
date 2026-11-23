//! miniq: a small in-memory message queue driven by one JSON command per line.

mod broker;
mod error;
mod handles;
mod queue;
mod request;
mod response;
mod types;

use std::io::{self, BufRead, Write};

pub use broker::Broker;
pub use error::{Error, ErrorCode};
pub use response::{Reply, write_response};

/// Answers every line of `input` with one response line on `output`, flushing after each.
pub fn run(mut input: impl BufRead, mut output: impl Write) -> io::Result<()> {
    let mut broker = Broker::new();
    let mut line = Vec::new();
    let mut response = Vec::new();
    loop {
        line.clear();
        if input.read_until(b'\n', &mut line)? == 0 {
            return Ok(());
        }
        if line.last() == Some(&b'\n') {
            line.pop();
        }
        response.clear();
        write_response(&broker.handle_line(&line), &mut response)?;
        output.write_all(&response)?;
        output.flush()?;
    }
}
