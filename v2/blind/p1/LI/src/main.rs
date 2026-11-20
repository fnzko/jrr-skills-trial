use std::io::{self, BufRead, Write};

use miniq::broker::Broker;

/// Reads one command per line from stdin and writes one response per command,
/// flushing after each so a client can wait for its answer.
fn main() -> io::Result<()> {
    let mut broker = Broker::new();
    let mut input = io::stdin().lock();
    let mut output = io::stdout().lock();
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
        writeln!(output, "{response}")?;
        output.flush()?;
    }
}
