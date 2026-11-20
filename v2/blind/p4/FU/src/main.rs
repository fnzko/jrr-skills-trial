//! The `miniq` binary: reads commands from stdin, one per line, and answers each on stdout.

use std::io::{self, BufRead, Write};

use miniq::{Service, handle_line};

fn main() -> io::Result<()> {
    let mut input = io::stdin().lock();
    let mut output = io::stdout().lock();
    let mut service = Service::new();
    // Lines are read as bytes so that invalid UTF-8 becomes an error response, not a crash.
    let mut line = Vec::new();
    loop {
        line.clear();
        if input.read_until(b'\n', &mut line)? == 0 {
            return Ok(());
        }
        if line.last() == Some(&b'\n') {
            line.pop();
        }
        let response = handle_line(&mut service, &line);
        writeln!(output, "{response}")?;
        // Flush before reading on, so an interactive client sees each answer immediately.
        output.flush()?;
    }
}
