//! miniq: a small in-memory message queue driven by JSON lines.

pub mod error;
pub mod ids;
pub mod queue;
pub mod request;
pub mod response;
pub mod service;

use std::io::{self, BufRead, Write};

use service::Service;

/// Reads commands from `input` and writes one reply per line to `output`,
/// flushing each reply before the next line is read.
pub fn run(mut input: impl BufRead, mut output: impl Write) -> io::Result<()> {
    let mut service = Service::default();
    let mut line = Vec::new();
    let mut reply = Vec::new();
    loop {
        line.clear();
        if input.read_until(b'\n', &mut line)? == 0 {
            return Ok(());
        }
        if line.last() == Some(&b'\n') {
            line.pop();
        }
        let result = request::parse(&line).and_then(|command| service.execute(command));
        reply.clear();
        response::write_reply(&mut reply, &result)?;
        reply.push(b'\n');
        output.write_all(&reply)?;
        output.flush()?;
    }
}
