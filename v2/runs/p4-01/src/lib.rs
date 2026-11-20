//! miniq: a small in-memory message queue driven by JSON lines.
//!
//! Each input line is one command and gets exactly one response line. All
//! state lives in memory and time only moves on `Tick`, so a run is fully
//! determined by its input.

mod broker;
mod error;
mod queue;
mod request;
mod store;
mod types;

use std::fmt::Display;
use std::io::{self, BufRead, Write};

use serde::{Serialize, Serializer};

use crate::broker::{Broker, Output};
use crate::error::{Error, ErrorCode};

/// Answers every line of `input` on `output` until end of input, flushing
/// each response before reading the next line.
///
/// A final line without `\n` counts unless it is empty. Command failures are
/// responses, not errors; the only errors are those of reading and writing.
pub fn run(mut input: impl BufRead, mut output: impl Write) -> io::Result<()> {
    let mut broker = Broker::default();
    let mut line = Vec::new();
    loop {
        line.clear();
        if input.read_until(b'\n', &mut line)? == 0 {
            return Ok(());
        }
        let command = line.strip_suffix(b"\n").unwrap_or(&line);
        let result = request::parse(command).and_then(|request| broker.execute(request));
        write_response(&mut output, &result)?;
        output.flush()?;
    }
}

fn write_response(output: &mut impl Write, result: &Result<Output<'_>, Error>) -> io::Result<()> {
    match result {
        Ok(output_members) => {
            serde_json::to_writer(
                &mut *output,
                &Success {
                    ok: true,
                    output: output_members,
                },
            )?;
        }
        Err(error) => {
            let failure = Failure {
                ok: false,
                error: FailureDetail {
                    code: error.code(),
                    message: error,
                },
            };
            serde_json::to_writer(&mut *output, &failure)?;
        }
    }
    output.write_all(b"\n")
}

#[derive(Serialize)]
struct Success<'r, 'a> {
    ok: bool,
    #[serde(flatten)]
    output: &'r Output<'a>,
}

#[derive(Serialize)]
struct Failure<'r> {
    ok: bool,
    error: FailureDetail<'r>,
}

#[derive(Serialize)]
struct FailureDetail<'r> {
    code: ErrorCode,
    #[serde(serialize_with = "serialize_display")]
    message: &'r Error,
}

fn serialize_display<S: Serializer>(
    value: &impl Display,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.collect_str(value)
}
