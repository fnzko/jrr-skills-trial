//! SendMessageBatch and DeleteMessageBatch (SPEC-feature section 2).

use std::collections::HashSet;

use serde_json::{Map, Value};

use super::{Broker, Members, members};
use crate::error::{Error, ErrorCode, Result};
use crate::request::Params;

const MAX_ENTRIES: usize = 10;
const MAX_ID_CHARS: usize = 80;

fn invalid_entry(message: &str) -> Error {
    Error::new(ErrorCode::InvalidBatchEntryId, message)
}

/// The batch rules 1 to 4: entry count, `Id` validity and distinctness.
/// Returns each entry's `Id` with its members.
fn check_entries(entries: &[Value]) -> Result<Vec<(&str, &Map<String, Value>)>> {
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
    let mut checked = Vec::with_capacity(entries.len());
    for entry in entries {
        let object = entry
            .as_object()
            .ok_or_else(|| invalid_entry("an entry is not an object"))?;
        let id = Params::new(object)
            .identifier("Id", MAX_ID_CHARS)
            .ok()
            .flatten()
            .ok_or_else(|| invalid_entry("an entry has no valid `Id`"))?;
        checked.push((id, object));
    }
    let mut seen = HashSet::new();
    if !checked.iter().all(|(id, _)| seen.insert(*id)) {
        return Err(Error::new(
            ErrorCode::BatchEntryIdsNotDistinct,
            "two entries have the same `Id`",
        ));
    }
    Ok(checked)
}

impl Broker {
    pub(super) fn send_message_batch(&mut self, params: Params<'_>) -> Result<Members> {
        self.run_batch(params, |broker, single| broker.send_message(single))
    }

    pub(super) fn delete_message_batch(&mut self, params: Params<'_>) -> Result<Members> {
        self.run_batch(params, |broker, single| broker.delete_message(single))
    }

    /// Validates the batch, then runs every entry as the single command
    /// `single` would run it, in order.
    fn run_batch(
        &mut self,
        params: Params<'_>,
        single: impl Fn(&mut Self, Params<'_>) -> Result<Members>,
    ) -> Result<Members> {
        params.require(&["QueueName", "Entries"])?;
        let name = params.string("QueueName")?.expect("required");
        let entries = params.array("Entries")?.expect("required");
        self.queue_mut(name)?;
        let entries = check_entries(entries)?;

        let mut successful = Vec::new();
        let mut failed = Vec::new();
        for (id, entry) in entries {
            // The entry's parameters plus the batch's queue; `Id` is ignored
            // by the single commands.
            let mut command = entry.clone();
            command.insert("QueueName".into(), Value::String(name.to_owned()));
            match single(self, Params::new(&command)) {
                Ok(mut result) => {
                    result.insert("Id".into(), Value::String(id.to_owned()));
                    successful.push(Value::Object(result));
                }
                Err(error) => {
                    failed.push(serde_json::json!({ "Id": id, "error": error.to_json() }));
                }
            }
        }
        Ok(members([
            ("Successful", Value::Array(successful)),
            ("Failed", Value::Array(failed)),
        ]))
    }
}
