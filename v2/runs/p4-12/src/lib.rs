//! miniq: a small in-memory message queue service.
//!
//! The service reads one JSON command per line and answers each with one JSON line (see `SPEC.md`).
//! [`Service`] turns a single line into a single response; [`run`] drives it over a byte stream.

mod dedup;
mod error;
mod queue;
mod request;
mod service;

use std::io::{self, BufRead, Write};

pub use error::{Error, ErrorCode};
pub use service::Service;

/// Answers every line of `input` on `output`, flushing after each response, until end of input.
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
        let response = service.handle_line(&line);
        output.write_all(response.as_bytes())?;
        output.write_all(b"\n")?;
        output.flush()?;
    }
}
