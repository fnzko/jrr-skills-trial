//! miniq: an in-memory message queue driven by JSON lines, on a virtual clock.
//!
//! Each input line is one command and gets one response line; see
//! [`Service::handle_line`] for a single command and [`run`] for a stream.

mod error;
mod queue;
mod request;
mod response;
mod service;
mod types;

use std::io::{self, BufRead, Write};

pub use error::{ApiError, ErrorCode};
pub use service::Service;

/// Answers every line of `input` on `output`, flushing after each response
/// so a client can wait for it before sending the next line.
///
/// # Errors
///
/// Fails when reading `input` or writing `output` fails.
pub fn run(mut input: impl BufRead, mut output: impl Write) -> io::Result<()> {
    let mut service = Service::new();
    let mut line = Vec::new();
    loop {
        line.clear();
        if input.read_until(b'\n', &mut line)? == 0 {
            return Ok(());
        }
        // Without a trailing `\n` this is a final line, and it is not empty.
        let command = line.strip_suffix(b"\n").unwrap_or(&line);
        let response = service.handle_line(command);
        output.write_all(response.as_bytes())?;
        output.write_all(b"\n")?;
        output.flush()?;
    }
}
