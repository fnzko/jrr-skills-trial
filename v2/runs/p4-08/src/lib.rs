//! miniq: an in-memory message queue driven by JSON commands, one per line.
//!
//! [`serve`] runs the whole protocol over any reader and writer; the binary
//! connects it to stdin and stdout.

pub mod broker;
pub mod error;
pub mod queue;
pub mod request;
pub mod response;
pub mod types;

use std::io::{self, BufRead, Write};

pub use broker::Broker;

/// Runs one command line, without its trailing `\n`, and returns the response
/// line, without its trailing `\n`.
pub fn handle_line(broker: &mut Broker, line: &[u8]) -> String {
    let result = request::parse(line).and_then(|request| broker.execute(request));
    response::render(&result)
}

/// Answers every line of `input` on `output` until end of input.
///
/// Each response is flushed before the next line is read, so a client can
/// wait for one answer before sending its next command.
///
/// # Errors
///
/// Fails only when reading `input` or writing `output` fails.
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
        writeln!(output, "{}", handle_line(&mut broker, &line))?;
        output.flush()?;
    }
}
