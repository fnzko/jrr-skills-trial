//! The response line of SPEC.md section 1: `{"ok": true}` with the result members of the
//! command, or `{"ok": false, "error": {...}}`.

use serde::{Serialize, Serializer};

use crate::error::Failure;
use crate::queue::{Counts, Delivered, QueueAttributes, Sent};
use crate::types::{Clock, QueueName};

/// The result members of a successful command.
#[derive(Debug, Serialize)]
#[serde(untagged, rename_all_fields = "PascalCase")]
pub enum Output {
    /// Commands whose success carries no result members.
    Done {},
    Attributes {
        attributes: QueueAttributes,
        counts: Counts,
    },
    QueueNames {
        queue_names: Vec<QueueName>,
    },
    Now {
        now: Clock,
    },
    Sent(Sent),
    Messages {
        messages: Vec<Delivered>,
    },
}

/// One response line.
#[derive(Debug)]
pub enum Response {
    Success(Output),
    Failure(Failure),
}

impl Response {
    pub fn from_result(result: Result<Output, Failure>) -> Response {
        match result {
            Ok(output) => Response::Success(output),
            Err(failure) => Response::Failure(failure),
        }
    }
}

impl Serialize for Response {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct SuccessLine<'a> {
            ok: bool,
            #[serde(flatten)]
            output: &'a Output,
        }
        #[derive(Serialize)]
        struct FailureLine<'a> {
            ok: bool,
            error: &'a Failure,
        }
        match self {
            Response::Success(output) => SuccessLine { ok: true, output }.serialize(serializer),
            Response::Failure(error) => FailureLine { ok: false, error }.serialize(serializer),
        }
    }
}
