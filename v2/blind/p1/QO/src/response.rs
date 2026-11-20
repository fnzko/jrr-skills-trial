//! What a command answers with; serialized as one JSON object per line.

use std::io::{self, Write};

use serde::Serialize;

use crate::error::Error;
use crate::queue::{Attributes, Counts, ReceivedMessage, Sent};
use crate::types::{Instant, QueueName};

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct Empty {}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Clock {
    pub now: Instant,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Messages {
    pub messages: Vec<ReceivedMessage>,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct QueueAttributes {
    pub attributes: Attributes,
    pub counts: Counts,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct QueueNames {
    pub queue_names: Vec<QueueName>,
}

/// The result members of a successful command.
#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum Outcome {
    Done(Empty),
    Clock(Clock),
    Sent(Sent),
    Messages(Messages),
    QueueAttributes(QueueAttributes),
    QueueNames(QueueNames),
}

#[derive(Serialize)]
struct Success<'a> {
    ok: bool,
    #[serde(flatten)]
    outcome: &'a Outcome,
}

#[derive(Serialize)]
struct Failure<'a> {
    ok: bool,
    error: &'a Error,
}

/// Writes the response line for `result` and flushes it.
pub fn write_response(out: &mut impl Write, result: &Result<Outcome, Error>) -> io::Result<()> {
    match result {
        Ok(outcome) => serde_json::to_writer(&mut *out, &Success { ok: true, outcome })?,
        Err(error) => serde_json::to_writer(&mut *out, &Failure { ok: false, error })?,
    }
    out.write_all(b"\n")?;
    out.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorCode;
    use serde_json::json;

    fn line(result: &Result<Outcome, Error>) -> serde_json::Value {
        let mut out = Vec::new();
        write_response(&mut out, result).unwrap();
        assert_eq!(out.last(), Some(&b'\n'));
        assert_eq!(out.iter().filter(|&&b| b == b'\n').count(), 1);
        serde_json::from_slice(&out).unwrap()
    }

    #[test]
    fn success_without_results_is_just_ok() {
        assert_eq!(line(&Ok(Outcome::Done(Empty {}))), json!({"ok": true}));
    }

    #[test]
    fn success_flattens_the_result_members() {
        let outcome = Outcome::Clock(Clock {
            now: Instant::START,
        });
        assert_eq!(line(&Ok(outcome)), json!({"ok": true, "Now": 0}));
    }

    #[test]
    fn failure_carries_code_and_message() {
        let error = Error::new(ErrorCode::InvalidRequest, "bad line");
        assert_eq!(
            line(&Err(error)),
            json!({"ok": false, "error": {"code": "InvalidRequest", "message": "bad line"}})
        );
    }
}
