use std::io::{self, BufRead, Write};

use miniq::Broker;

/// Reads commands from stdin and writes one response line per command.
fn main() -> io::Result<()> {
    let mut input = io::stdin().lock();
    let mut output = io::stdout().lock();
    let mut broker = Broker::new();
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
