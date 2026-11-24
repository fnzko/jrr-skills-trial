//! The line protocol: one JSON command per input line, one JSON response
//! per command.

use std::io::{self, BufRead, Write};

use serde::Serialize;
use serde_json::Value;

use crate::command::Command;
use crate::error::{Error, ErrorCode};
use crate::reply::Reply;
use crate::service::Service;

#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum Response {
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
            Ok(reply) => Response::Success { ok: true, reply },
            Err(error) => Response::Failure { ok: false, error },
        }
    }
}

impl Service {
    /// Runs one input line, which may be blank or malformed.
    pub fn respond(&mut self, line: &[u8]) -> Response {
        self.run_line(line).into()
    }

    fn run_line(&mut self, line: &[u8]) -> Result<Reply, Error> {
        let request: Value = serde_json::from_slice(line).map_err(|cause| {
            Error::new(
                ErrorCode::InvalidRequest,
                format!("line is not valid UTF-8 JSON: {cause}"),
            )
        })?;
        let Value::Object(members) = &request else {
            return Err(Error::new(
                ErrorCode::InvalidRequest,
                "line is not a JSON object",
            ));
        };
        self.execute(Command::parse(members)?)
    }
}

/// Answers every line of `input` on `output`, flushing after each response.
pub fn serve(mut input: impl BufRead, mut output: impl Write) -> io::Result<()> {
    let mut service = Service::new();
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
