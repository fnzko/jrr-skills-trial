//! Batch message commands (SPEC-feature section 2).

use std::collections::HashSet;

use serde_json::{Map, Value, json};

use super::{Broker, Members};
use crate::error::{Error, ErrorCode, Result};
use crate::naming;
use crate::request::Params;

const MAX_ENTRIES: usize = 10;
const MAX_ENTRY_ID_CHARS: usize = 80;

fn invalid_entry_id() -> Error {
    Error::new(
        ErrorCode::InvalidBatchEntryId,
        "each entry must be an object with an `Id` of 1 to 80 characters from A-Z a-z 0-9 _ -",
    )
}

/// The `Id` of every entry, in order, after the batch-level rules 1 to 4.
fn entry_ids(entries: &[Value]) -> Result<Vec<&str>> {
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
    let ids = entries
        .iter()
        .map(|entry| {
            entry
                .get("Id")
                .and_then(Value::as_str)
                .filter(|id| naming::is_identifier(id, MAX_ENTRY_ID_CHARS))
                .ok_or_else(invalid_entry_id)
        })
        .collect::<Result<Vec<_>>>()?;
    let mut seen = HashSet::new();
    if !ids.iter().all(|id| seen.insert(*id)) {
        return Err(Error::new(
            ErrorCode::BatchEntryIdsNotDistinct,
            "two entries have the same `Id`",
        ));
    }
    Ok(ids)
}

impl Broker {
    pub(super) fn send_message_batch(&mut self, params: Params<'_>) -> Result<Members> {
        self.run_batch(params, Self::send_message)
    }

    pub(super) fn delete_message_batch(&mut self, params: Params<'_>) -> Result<Members> {
        self.run_batch(params, Self::delete_message)
    }

    /// Validates the batch, then runs `single` for each entry as if it were
    /// a separate command on the same queue.
    fn run_batch(
        &mut self,
        params: Params<'_>,
        single: fn(&mut Self, Params<'_>) -> Result<Members>,
    ) -> Result<Members> {
        params.require(&["QueueName", "Entries"])?;
        let name = params.string("QueueName")?.expect("required");
        let entries = params.array("Entries")?.expect("required");
        self.queue_mut(name)?;
        let ids = entry_ids(entries)?;

        let mut successful = Vec::new();
        let mut failed = Vec::new();
        for (id, entry) in ids.into_iter().zip(entries) {
            // `entry_ids` has checked that the entry is an object.
            let mut command: Map<String, Value> = entry
                .as_object()
                .expect("entry_ids accepts objects only")
                .clone();
            command.insert("QueueName".into(), json!(name));
            match single(self, Params::new(&command)) {
                Ok(mut result) => {
                    result.insert("Id".into(), json!(id));
                    successful.push(Value::Object(result));
                }
                Err(Error { code, message }) => failed.push(json!({
                    "Id": id,
                    "error": { "code": code.as_str(), "message": message },
                })),
            }
        }
        Ok(super::members([
            ("Successful", Value::Array(successful)),
            ("Failed", Value::Array(failed)),
        ]))
    }
}
