//! miniq: a small in-memory message queue driven by one JSON command per line.

pub mod attributes;
pub mod error;
pub mod queue;
pub mod reply;
pub mod request;
pub mod service;
pub mod types;

use std::io::{self, BufRead, Write};

use reply::Response;
use service::Service;

/// Runs the line protocol: one response line, flushed, for every input line, until end of input.
pub fn run(mut input: impl BufRead, mut output: impl Write) -> io::Result<()> {
    let mut service = Service::new();
    let mut line = Vec::new();
    loop {
        line.clear();
        if input.read_until(b'\n', &mut line)? == 0 {
            return Ok(());
        }
        if line.last() == Some(&b'\n') {
            line.pop();
        }
        let response = Response::from(service.handle_line(&line));
        serde_json::to_writer(&mut output, &response)?;
        output.write_all(b"\n")?;
        output.flush()?;
    }
}
