//! Batch commands (SPEC-feature section 2).

use std::collections::HashSet;

use serde_json::{Value, json};

use super::{Broker, Members, members};
use crate::error::{Error, ErrorCode, Result};
use crate::request::Params;

const MAX_ENTRIES: usize = 10;
const MAX_ENTRY_ID_CHARS: usize = 80;

/// Handles one entry of a batch like the single command would.
type EntryHandler = fn(&mut Broker, &str, Params<'_>) -> Result<Members>;

/// Applies the numbered rules 1 to 4 and returns each entry with its `Id`.
fn parse_entries(entries: &[Value]) -> Result<Vec<(&str, Params<'_>)>> {
    if entries.is_empty() {
        return Err(Error::new(
            ErrorCode::EmptyBatchRequest,
            "`Entries` has no entry",
        ));
    }
    if entries.len() > MAX_ENTRIES {
        return Err(Error::new(
            ErrorCode::TooManyEntriesInBatchRequest,
            "`Entries` has more than 10 entries",
        ));
    }
    let invalid = || {
        Error::new(
            ErrorCode::InvalidBatchEntryId,
            "an entry must be an object with a valid `Id`",
        )
    };
    let parsed = entries
        .iter()
        .map(|entry| {
            let params = entry.as_object().map(Params::new).ok_or_else(invalid)?;
            let id = params
                .identifier("Id", MAX_ENTRY_ID_CHARS)
                .map_err(|_| invalid())?
                .ok_or_else(invalid)?;
            Ok((id, params))
        })
        .collect::<Result<Vec<_>>>()?;
    let mut seen = HashSet::new();
    if parsed.iter().any(|(id, _)| !seen.insert(*id)) {
        return Err(Error::new(
            ErrorCode::BatchEntryIdsNotDistinct,
            "two entries have the same `Id`",
        ));
    }
    Ok(parsed)
}

impl Broker {
    pub(super) fn send_message_batch(&mut self, params: Params<'_>) -> Result<Members> {
        self.run_batch(params, Self::send_entry)
    }

    pub(super) fn delete_message_batch(&mut self, params: Params<'_>) -> Result<Members> {
        self.run_batch(params, Self::delete_entry)
    }

    fn run_batch(&mut self, params: Params<'_>, handle: EntryHandler) -> Result<Members> {
        params.require(&["QueueName", "Entries"])?;
        let name = params
            .string("QueueName")?
            .expect("present: checked by `require` above");
        let entries = params
            .array("Entries")?
            .expect("present: checked by `require` above");
        self.queue_mut(name)?;
        let entries = parse_entries(entries)?;

        let mut successful = Vec::new();
        let mut failed = Vec::new();
        for (id, entry) in entries {
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
