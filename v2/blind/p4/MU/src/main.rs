use std::io::{self, BufRead, Write};

use miniq::Broker;

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
        // A read without the newline is the final line, and it is non-empty
        // because the read returned at least one byte.
        if line.last() == Some(&b'\n') {
            line.pop();
        }
        serde_json::to_writer(&mut output, &broker.handle_line(&line))?;
        output.write_all(b"\n")?;
        output.flush()?;
    }
}
