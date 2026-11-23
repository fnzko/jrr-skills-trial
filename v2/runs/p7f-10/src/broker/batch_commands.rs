//! Batch commands (SPEC-feature section 2).

use std::collections::HashSet;

use serde_json::{Map, Value, json};

use super::{Broker, Members, error_json, members};
use crate::error::{Error, ErrorCode, Result};
use crate::naming;
use crate::request::Params;

const MAX_ENTRIES: usize = 10;
const MAX_ID_CHARS: usize = 80;

/// Checks the batch rules 1 to 4 and returns each entry's id and members.
fn parse_entries(entries: &[Value]) -> Result<Vec<(&str, &Map<String, Value>)>> {
    if entries.is_empty() {
        return Err(Error::new(
            ErrorCode::EmptyBatchRequest,
            "`Entries` has no entry",
        ));
    }
    if entries.len() > MAX_ENTRIES {
        return Err(Error::new(
            ErrorCode::TooManyEntriesInBatchRequest,
            format!("`Entries` has more than {MAX_ENTRIES} entries"),
        ));
    }
    let mut parsed = Vec::with_capacity(entries.len());
    for entry in entries {
        let id = entry
            .as_object()
            .and_then(|o| Some((o.get("Id")?.as_str()?, o)))
            .filter(|(id, _)| naming::is_identifier(id, MAX_ID_CHARS));
        match id {
            Some(pair) => parsed.push(pair),
            None => {
                return Err(Error::new(
                    ErrorCode::InvalidBatchEntryId,
                    "every entry must be an object with an `Id` of 1 to 80 characters from A-Z a-z 0-9 _ -",
                ));
            }
        }
    }
    let mut seen = HashSet::new();
    if !parsed.iter().all(|(id, _)| seen.insert(*id)) {
        return Err(Error::new(
            ErrorCode::BatchEntryIdsNotDistinct,
            "entry ids must be distinct",
        ));
    }
    Ok(parsed)
}

impl Broker {
    pub(super) fn send_message_batch(&mut self, params: Params<'_>) -> Result<Members> {
        self.run_batch(params, Self::send_to_queue)
    }

    pub(super) fn delete_message_batch(&mut self, params: Params<'_>) -> Result<Members> {
        self.run_batch(params, Self::delete_in_queue)
    }

    /// Validates the request, then runs `handle` for each entry in order.
    fn run_batch(
        &mut self,
        params: Params<'_>,
        handle: fn(&mut Self, &str, Params<'_>) -> Result<Members>,
    ) -> Result<Members> {
        params.require(&["QueueName", "Entries"])?;
        let name = params.string("QueueName")?.expect("required");
        let entries = params.array("Entries")?.expect("required");
        self.queue_mut(name)?;
        let entries = parse_entries(entries)?;

        let mut successful = Vec::new();
        let mut failed = Vec::new();
        for (id, entry) in entries {
            match handle(self, name, Params::new(entry)) {
                Ok(mut result) => {
                    result.insert("Id".into(), json!(id));
                    successful.push(Value::Object(result));
                }
                Err(error) => failed.push(json!({ "Id": id, "error": error_json(&error) })),
            }
        }
        Ok(members([
            ("Successful", Value::Array(successful)),
            ("Failed", Value::Array(failed)),
        ]))
    }
}
