//! The line protocol: one request line in, one response line out.

use std::io::{self, BufRead, Write};

use serde::Serialize;

use crate::broker::Broker;
use crate::error::CommandError;
use crate::reply::Reply;
use crate::request;

#[derive(Serialize)]
struct Success<'a> {
    ok: bool,
    #[serde(flatten)]
    reply: &'a Reply,
}

#[derive(Serialize)]
struct Failure<'a> {
    ok: bool,
    error: &'a CommandError,
}

/// Runs one request line against the broker.
fn execute(broker: &mut Broker, line: &[u8]) -> Result<Reply, CommandError> {
    let request = request::parse_line(line)?;
    broker.execute(request::parse_command(&request)?)
}

/// Writes the response line for `outcome` to `buffer`, without flushing it anywhere.
fn encode(outcome: &Result<Reply, CommandError>, buffer: &mut Vec<u8>) -> serde_json::Result<()> {
    match outcome {
        Ok(reply) => serde_json::to_writer(&mut *buffer, &Success { ok: true, reply })?,
        Err(error) => serde_json::to_writer(&mut *buffer, &Failure { ok: false, error })?,
    }
    buffer.push(b'\n');
    Ok(())
}

/// Serves requests from `input` until it ends, answering each line on `output` before the
/// next one is read.
pub fn run(mut input: impl BufRead, mut output: impl Write) -> io::Result<()> {
    let mut broker = Broker::new();
    let mut line = Vec::new();
    let mut response = Vec::new();
    loop {
        line.clear();
        if input.read_until(b'\n', &mut line)? == 0 {
            return Ok(());
        }
        if line.last() == Some(&b'\n') {
            line.pop();
        }
        response.clear();
        encode(&execute(&mut broker, &line), &mut response)?;
        output.write_all(&response)?;
        output.flush()?;
    }
}
