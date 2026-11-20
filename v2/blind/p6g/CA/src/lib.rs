//! miniq: a small in-memory message queue driven by JSON lines.

pub mod command;
pub mod error;
pub mod parse;
pub mod queue;
pub mod reply;
pub mod service;
pub mod types;

use std::io::{self, BufRead, Write};

use service::Service;

/// Answers every line of `input` with one line on `output`, flushing after
/// each response, until the end of input.
pub fn run(mut input: impl BufRead, mut output: impl Write) -> io::Result<()> {
    let mut service = Service::default();
    let mut line = Vec::new();
    loop {
        line.clear();
        if input.read_until(b'\n', &mut line)? == 0 {
            return Ok(());
        }
        if line.last() == Some(&b'\n') {
            line.pop();
        }
        let result = parse::parse(&line).and_then(|command| service.execute(command));
        writeln!(output, "{}", reply::render(result))?;
        output.flush()?;
    }
}
