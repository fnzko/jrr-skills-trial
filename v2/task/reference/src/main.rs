// The base-only build (no "extension" feature) keeps the extension's data model unused.
#![cfg_attr(not(feature = "extension"), allow(dead_code))]

mod error;
mod params;
mod queue;
mod service;

use std::io::{self, BufRead, Write};

use service::Service;

fn main() -> io::Result<()> {
    let mut input = io::stdin().lock();
    let mut output = io::stdout().lock();
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
        serde_json::to_writer(&mut output, &service.respond(&line))?;
        output.write_all(b"\n")?;
        output.flush()?;
    }
}
