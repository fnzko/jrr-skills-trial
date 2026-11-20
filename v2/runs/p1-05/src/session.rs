//! The line protocol: one command per input line, one response per command.

use std::io::{self, BufRead, Write};

use serde::Serialize;

use crate::broker::{Broker, Output};
use crate::error::Error;
use crate::protocol;

#[derive(Serialize)]
struct Success<'a> {
    ok: bool,
    #[serde(flatten)]
    output: &'a Output,
}

#[derive(Serialize)]
struct Failure<'a> {
    ok: bool,
    error: ErrorBody<'a>,
}

#[derive(Serialize)]
struct ErrorBody<'a> {
    code: &'a str,
    message: &'a str,
}

fn render(result: &Result<Output, Error>, out: &mut Vec<u8>) -> serde_json::Result<()> {
    match result {
        Ok(output) => serde_json::to_writer(&mut *out, &Success { ok: true, output }),
        Err(error) => serde_json::to_writer(
            &mut *out,
            &Failure {
                ok: false,
                error: ErrorBody {
                    code: error.code().as_str(),
                    message: error.message(),
                },
            },
        ),
    }
}

/// Answers every line of `input` on `output`, flushing after each response,
/// until the input ends.
pub fn serve(mut input: impl BufRead, mut output: impl Write) -> io::Result<()> {
    let mut broker = Broker::default();
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
        let result = protocol::parse(&line).and_then(|command| broker.execute(command));
        response.clear();
        render(&result, &mut response)?;
        response.push(b'\n');
        output.write_all(&response)?;
        output.flush()?;
    }
}
