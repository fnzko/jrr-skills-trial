//! miniq: a small in-memory message queue driven by JSON commands.

pub mod attributes;
pub mod broker;
pub mod error;
pub mod queue;
pub mod request;
pub mod types;

use std::io::{self, BufRead, Write};

use serde::Serialize;

use crate::broker::{Broker, Reply};
use crate::error::Error;

/// The line written for one command.
#[derive(Serialize)]
#[serde(untagged)]
enum Response {
    Success {
        ok: bool,
        #[serde(flatten)]
        reply: Reply,
    },
    Failure {
        ok: bool,
        error: Error,
    },
}

impl From<Result<Reply, Error>> for Response {
    fn from(result: Result<Reply, Error>) -> Self {
        match result {
            Ok(reply) => Self::Success { ok: true, reply },
            Err(error) => Self::Failure { ok: false, error },
        }
    }
}

/// Answers every line of `input` with one response line on `output`, flushing after
/// each, until the input ends.
pub fn run(mut input: impl BufRead, mut output: impl Write) -> io::Result<()> {
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
        let result = request::parse(&line).and_then(|command| broker.execute(command));
        serde_json::to_writer(&mut output, &Response::from(result))?;
        output.write_all(b"\n")?;
        output.flush()?;
    }
}
