//! The line protocol of SPEC.md section 1: one input line is one command, and it gets exactly
//! one response line, written and flushed before the next input line is read.

use std::io::{self, BufRead, Write};

use crate::broker::Broker;
use crate::response::Response;

/// Serves commands from `input` until its end, writing responses to `output`.
pub fn serve<R: BufRead, W: Write>(mut input: R, mut output: W) -> io::Result<()> {
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
        let response = Response::from_result(broker.execute_line(&line));
        serde_json::to_writer(&mut output, &response).map_err(io::Error::other)?;
        output.write_all(b"\n")?;
        output.flush()?;
    }
}
