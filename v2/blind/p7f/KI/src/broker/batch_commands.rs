//! Batch message commands (SPEC-feature section 2).

use std::collections::HashSet;

use serde_json::{Map, Value, json};

use super::{Broker, Members, members};
use crate::error::{Error, ErrorCode, Result};
use crate::naming;
use crate::request::Params;

const MAX_ENTRIES: usize = 10;
const MAX_ENTRY_ID_CHARS: usize = 80;

/// A batch entry whose `Id` has been validated.
struct Entry<'a> {
    id: &'a str,
    members: &'a Map<String, Value>,
}

fn invalid_entry_id(message: &str) -> Error {
    Error::new(ErrorCode::InvalidBatchEntryId, message)
}

/// Applies numbered rules 1 to 4 to the `Entries` of a batch.
fn parse_entries(entries: &[Value]) -> Result<Vec<Entry<'_>>> {
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
    let parsed = entries
        .iter()
        .map(|entry| {
            let members = entry
                .as_object()
                .ok_or_else(|| invalid_entry_id("an entry is not an object"))?;
            let id = Params::new(members)
                .string("Id")
                .ok()
                .flatten()
                .filter(|id| naming::is_identifier(id, MAX_ENTRY_ID_CHARS))
                .ok_or_else(|| {
                    invalid_entry_id(
                        "an entry `Id` must be 1 to 80 characters from A-Z a-z 0-9 _ -",
                    )
                })?;
            Ok(Entry { id, members })
        })
        .collect::<Result<Vec<_>>>()?;
    let mut seen = HashSet::with_capacity(parsed.len());
    if !parsed.iter().all(|entry| seen.insert(entry.id)) {
        return Err(Error::new(
            ErrorCode::BatchEntryIdsNotDistinct,
            "two entries have the same `Id`",
        ));
    }
    Ok(parsed)
}

impl Broker {
    pub(super) fn send_message_batch(&mut self, params: Params<'_>) -> Result<Members> {
        self.run_batch(params, Broker::send_message)
    }

    pub(super) fn delete_message_batch(&mut self, params: Params<'_>) -> Result<Members> {
        self.run_batch(params, Broker::delete_message)
    }

    /// Runs each entry as the single command `single` would run it, with the
    /// batch's `QueueName`, and sorts the outcomes into `Successful` and
    /// `Failed`.
    fn run_batch(
        &mut self,
        params: Params<'_>,
        single: fn(&mut Broker, Params<'_>) -> Result<Members>,
    ) -> Result<Members> {
        params.require(&["QueueName", "Entries"])?;
        let name = params.string("QueueName")?.expect("required");
        let entries = params.array("Entries")?.expect("required");
        self.queue_mut(name)?;
        let entries = parse_entries(entries)?;

        let mut successful = Vec::new();
        let mut failed = Vec::new();
        for entry in entries {
            let mut command = entry.members.clone();
            command.insert("QueueName".to_owned(), json!(name));
            match single(self, Params::new(&command)) {
                Ok(mut result) => {
                    result.insert("Id".to_owned(), json!(entry.id));
                    successful.push(Value::Object(result));
                }
                Err(Error { code, message }) => failed.push(json!({
                    "Id": entry.id,
                    "error": { "code": code.as_str(), "message": message },
                })),
            }
        }
        Ok(members([
            ("Successful", Value::Array(successful)),
            ("Failed", Value::Array(failed)),
        ]))
    }
}
