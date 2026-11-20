use serde::Serialize;

use crate::error::Error;
use crate::queue::{Attributes, Counts, Delivery, Sent};

/// The result members of a successful command.
#[derive(Debug, Serialize)]
#[serde(untagged, rename_all_fields = "PascalCase")]
pub(crate) enum Outcome {
    Done {},
    Now {
        now: u64,
    },
    Queue {
        attributes: Attributes,
        counts: Counts,
    },
    QueueNames {
        queue_names: Vec<String>,
    },
    Sent(Sent),
    Messages {
        messages: Vec<Delivery>,
    },
}

#[derive(Serialize)]
struct Success {
    ok: bool,
    #[serde(flatten)]
    outcome: Outcome,
}

#[derive(Serialize)]
struct Failure {
    ok: bool,
    error: Error,
}

/// One response line, without the trailing newline.
pub(crate) fn render(result: Result<Outcome, Error>) -> String {
    let line = match result {
        Ok(outcome) => serde_json::to_string(&Success { ok: true, outcome }),
        Err(error) => serde_json::to_string(&Failure { ok: false, error }),
    };
    line.expect("responses hold only strings, booleans and integers, which always serialize")
}
