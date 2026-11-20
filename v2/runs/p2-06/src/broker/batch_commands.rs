//! SendMessageBatch and DeleteMessageBatch (SPEC-feature section 2).

use std::collections::HashSet;

use serde_json::{Map, Value, json};

use super::{Broker, Members, members};
use crate::error::{Error, ErrorCode, Result};
use crate::request::Params;

const MAX_ENTRIES: usize = 10;
const MAX_ENTRY_ID_CHARS: usize = 80;

fn invalid_entry_id(index: usize) -> Error {
    Error::new(
        ErrorCode::InvalidBatchEntryId,
        format!("entry {index} is not an object with a valid `Id`"),
    )
}

/// Applies the numbered rules 1 to 4 and returns each entry's id with its
/// parameters.
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
    for (index, entry) in entries.iter().enumerate() {
        let map = entry.as_object().ok_or_else(|| invalid_entry_id(index))?;
        let id = Params::new(map)
            .identifier("Id", MAX_ENTRY_ID_CHARS)
            .ok()
            .flatten()
            .ok_or_else(|| invalid_entry_id(index))?;
        parsed.push((id, map));
    }
    let mut seen = HashSet::new();
    if let Some((id, _)) = parsed.iter().find(|(id, _)| !seen.insert(*id)) {
        return Err(Error::new(
            ErrorCode::BatchEntryIdsNotDistinct,
            format!("entry id `{id}` is used twice"),
        ));
    }
    Ok(parsed)
}

impl Broker {
    pub(super) fn send_message_batch(&mut self, params: Params<'_>) -> Result<Members> {
        self.run_batch(params, Self::send_to)
    }

    pub(super) fn delete_message_batch(&mut self, params: Params<'_>) -> Result<Members> {
        self.run_batch(params, Self::delete_from)
    }

    /// Validates the batch, then runs `handle` for each entry in order.
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
                Err(error) => failed.push(json!({ "Id": id, "error": error.to_json() })),
            }
        }
        Ok(members([
            ("Successful", Value::Array(successful)),
            ("Failed", Value::Array(failed)),
        ]))
    }
}
