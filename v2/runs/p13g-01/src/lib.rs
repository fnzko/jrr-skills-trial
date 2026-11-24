//! miniq: a small in-memory message queue driven by JSON commands, one per line.

pub mod broker;
pub mod error;
pub mod queue;
pub mod request;
pub mod response;
pub mod types;

use std::io::{self, BufRead, Write};

use broker::Broker;
use response::Reply;

/// Answers every line of `input` with one line on `output`, flushing after each answer,
/// until the input ends.
pub fn run(mut input: impl BufRead, mut output: impl Write) -> io::Result<()> {
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
        let result = request::parse(&line).and_then(|request| broker.execute(request));
        serde_json::to_writer(&mut output, &Reply(&result))?;
        output.write_all(b"\n")?;
        output.flush()?;
    }
}
