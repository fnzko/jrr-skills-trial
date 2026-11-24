//! The JSON written for each command.

use serde::Serialize;

use crate::error::Error;
use crate::queue::{AttributesView, Counts, Delivered, Sent};
use crate::types::{QueueName, Timestamp};

/// The result members of a successful command.
#[derive(Debug, Serialize)]
#[serde(untagged, rename_all_fields = "PascalCase")]
pub enum Output {
    Empty {},
    Now {
        now: Timestamp,
    },
    Attributes {
        attributes: AttributesView,
        counts: Counts,
    },
    QueueNames {
        queue_names: Vec<QueueName>,
    },
    Sent(Sent),
    Messages {
        messages: Vec<Delivered>,
    },
}

/// A command's outcome as the one line of JSON that answers it.
#[derive(Debug)]
pub struct Reply<'a>(pub &'a Result<Output, Error>);

#[derive(Serialize)]
struct Success<'a> {
    ok: bool,
    #[serde(flatten)]
    output: &'a Output,
}

#[derive(Serialize)]
struct Failure<'a> {
    ok: bool,
    error: &'a Error,
}

impl Serialize for Reply<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self.0 {
            Ok(output) => Success { ok: true, output }.serialize(serializer),
            Err(error) => Failure { ok: false, error }.serialize(serializer),
        }
    }
}
