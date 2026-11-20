use std::io::{self, BufRead, Write};

use anyhow::Context;
use miniq::Service;

fn main() -> anyhow::Result<()> {
    let mut service = Service::new();
    let mut input = io::stdin().lock();
    let mut output = io::stdout().lock();
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
        let response = service.handle_line(&line);
        output
            .write_all(response.as_bytes())
            .and_then(|()| output.write_all(b"\n"))
            .and_then(|()| output.flush())
            .context("writing stdout")?;
    }
}
