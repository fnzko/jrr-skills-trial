use std::io::{self, BufRead, Write};

use anyhow::Context;
use miniq::Broker;

fn main() -> anyhow::Result<()> {
    let mut input = io::stdin().lock();
    let mut output = io::stdout().lock();
    let mut broker = Broker::default();
    let mut line = Vec::new();

    loop {
        line.clear();
        if input
            .read_until(b'\n', &mut line)
            .context("reading stdin")?
            == 0
        {
            return Ok(());
        }
        if line.last() == Some(&b'\n') {
            line.pop();
        }
        let mut response = broker.handle_line(&line);
        response.push('\n');
        // Each response must reach the reader before the next line is read.
        output
            .write_all(response.as_bytes())
            .and_then(|()| output.flush())
            .context("writing to stdout")?;
    }
}
