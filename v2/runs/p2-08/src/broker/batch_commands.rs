//! Batch commands (SPEC-feature section 2).

use std::collections::HashSet;

use serde_json::{Value, json};

use super::{Broker, Members, members};
use crate::error::{Error, ErrorCode, Result};
use crate::naming;
use crate::request::Params;

const MAX_ENTRIES: usize = 10;
const MAX_ENTRY_ID_CHARS: usize = 80;

fn batch_error(code: ErrorCode, message: &str) -> Error {
    Error::new(code, message)
}

/// Checks the numbered batch rules 1 to 4 and returns each entry's `Id` with
/// its parameters.
fn parse_entries(entries: &[Value]) -> Result<Vec<(&str, Params<'_>)>> {
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
            let params = entry.as_object().map(Params::new);
            // An `Id` of the wrong type is an invalid entry id, not a type error.
            let id = params
                .and_then(|p| p.string("Id").ok().flatten())
                .filter(|id| naming::is_identifier(id, MAX_ENTRY_ID_CHARS));
            match (id, params) {
                (Some(id), Some(params)) => Ok((id, params)),
                _ => Err(batch_error(
                    ErrorCode::InvalidBatchEntryId,
                    "every entry must be an object with an `Id` of 1 to 80 characters from A-Z a-z 0-9 _ -",
                )),
            }
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

impl Broker {
    pub(super) fn send_message_batch(&mut self, params: Params<'_>) -> Result<Members> {
        self.run_batch(params, Self::send_to_queue)
    }

    pub(super) fn delete_message_batch(&mut self, params: Params<'_>) -> Result<Members> {
        self.run_batch(params, Self::delete_from_queue)
    }

    /// Checks the request as a whole, then handles the entries in order with
    /// `handle`, which does what the single command does from its step 2 on.
    fn run_batch(
        &mut self,
        params: Params<'_>,
        handle: fn(&mut Self, &str, Params<'_>) -> Result<Members>,
    ) -> Result<Members> {
        params.require(&["QueueName", "Entries"])?;
        let name = params.string("QueueName")?.expect("required");
        let entries = params.array("Entries")?.expect("required");
        self.queue_mut(name)?;

        let mut successful = Vec::new();
        let mut failed = Vec::new();
        for (id, entry) in parse_entries(entries)? {
            match handle(self, name, entry) {
                Ok(mut result) => {
                    result.insert("Id".into(), json!(id));
                    successful.push(Value::Object(result));
                }
                Err(error) => failed.push(json!({ "Id": id, "error": error.to_json() })),
            }
        }
        Ok(members([
            ("Successful", Value::Array(successful)),
            ("Failed", Value::Array(failed)),
        ]))
    }
}
