//! Batch commands (SPEC-feature section 2).

use std::collections::HashSet;

use serde_json::{Map, Value, json};

use super::{Broker, Members, members};
use crate::error::{Error, ErrorCode, Result};
use crate::naming;
use crate::request::Params;

const MAX_ENTRIES: usize = 10;
const MAX_ENTRY_ID_CHARS: usize = 80;

fn batch_error(code: ErrorCode, message: &str) -> Error {
    Error::new(code, message)
}

/// The object of a batch entry that has already passed [`entry_objects`].
type EntryObject<'a> = &'a Map<String, Value>;

/// The checks that precede the handling of entries (rules 1 to 4), returning
/// each entry's `Id` with its object.
fn entry_objects(entries: &[Value]) -> Result<Vec<(&str, EntryObject<'_>)>> {
    if entries.is_empty() {
        return Err(batch_error(
            ErrorCode::EmptyBatchRequest,
            "`Entries` has no entry",
        ));
    }
    if entries.len() > MAX_ENTRIES {
        return Err(batch_error(
            ErrorCode::TooManyEntriesInBatchRequest,
            "`Entries` has more than 10 entries",
        ));
    }
    let parsed = entries
        .iter()
        .map(|entry| {
            let object = entry.as_object();
            object
                .and_then(|o| o.get("Id"))
                .and_then(Value::as_str)
                .filter(|id| naming::is_identifier(id, MAX_ENTRY_ID_CHARS))
                .zip(object)
                .ok_or_else(|| {
                    batch_error(
                        ErrorCode::InvalidBatchEntryId,
                        "an entry is not an object or has no valid `Id`",
                    )
                })
        })
        .collect::<Result<Vec<_>>>()?;
    let mut seen = HashSet::new();
    if !parsed.iter().all(|(id, _)| seen.insert(*id)) {
        return Err(batch_error(
            ErrorCode::BatchEntryIdsNotDistinct,
            "two entries have the same `Id`",
        ));
    }
    Ok(parsed)
}

/// Builds the `Successful` and `Failed` members from per-entry outcomes.
fn batch_result(outcomes: Vec<(&str, Result<Members>)>) -> Members {
    let mut successful = Vec::new();
    let mut failed = Vec::new();
    for (id, outcome) in outcomes {
        match outcome {
            Ok(mut result) => {
                result.insert("Id".into(), json!(id));
                successful.push(Value::Object(result));
            }
            Err(error) => failed.push(json!({ "Id": id, "error": error.to_json() })),
        }
    }
    members([
        ("Successful", Value::Array(successful)),
        ("Failed", Value::Array(failed)),
    ])
}

impl Broker {
    pub(super) fn send_message_batch(&mut self, params: Params<'_>) -> Result<Members> {
        self.run_batch(params, |broker, name, entry| {
            entry
                .require(&["Body"])
                .and_then(|()| broker.send_entry(name, entry))
        })
    }

    pub(super) fn delete_message_batch(&mut self, params: Params<'_>) -> Result<Members> {
        self.run_batch(params, |broker, name, entry| {
            entry
                .require(&["ReceiptHandle"])
                .and_then(|()| broker.delete_entry(name, entry))
        })
    }

    /// Checks the request, then handles the entries in order with `handle`,
    /// each one seeing the effects of those before it.
    fn run_batch(
        &mut self,
        params: Params<'_>,
        mut handle: impl FnMut(&mut Self, &str, Params<'_>) -> Result<Members>,
    ) -> Result<Members> {
        params.require(&["QueueName", "Entries"])?;
        let name = params.string("QueueName")?.expect("required");
        let entries = params.array("Entries")?.expect("required");
        self.queue_mut(name)?;
        let entries = entry_objects(entries)?;
        let outcomes = entries
            .into_iter()
            .map(|(id, object)| (id, handle(self, name, Params::new(object))))
            .collect();
        Ok(batch_result(outcomes))
    }
}
