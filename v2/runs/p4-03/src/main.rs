//! The miniq binary: reads command lines from stdin and writes one response line for each, in
//! order. Each response is flushed before the next line is read, so a client can wait for it.

use miniq::broker::Broker;
use std::io::{self, BufRead, Write};

fn main() -> io::Result<()> {
    let mut input = io::stdin().lock();
    let mut output = io::stdout().lock();
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
        let reply = broker.handle_line(&line);
        serde_json::to_writer(&mut output, &reply)?;
        output.write_all(b"\n")?;
        output.flush()?;
    }
}
