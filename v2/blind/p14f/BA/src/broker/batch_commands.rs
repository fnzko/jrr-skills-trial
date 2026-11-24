//! Batch commands (feature spec section 2).

use std::collections::HashSet;

use serde_json::{Value, json};

use super::{Broker, Members, error_json, members};
use crate::error::{Error, ErrorCode, Result};
use crate::request::Params;

const MAX_ENTRIES: usize = 10;
const MAX_ENTRY_ID_CHARS: usize = 80;

/// Handles one entry like the single command does from step 3 on.
type EntryHandler = fn(&mut Broker, &str, Params<'_>) -> Result<Members>;

fn invalid_entry_id(index: usize) -> Error {
    Error::new(
        ErrorCode::InvalidBatchEntryId,
        format!("entry {index} is not an object with a valid `Id`"),
    )
}

/// Checks rules 1 to 4 and returns each entry with its `Id`.
fn checked_entries<'a>(entries: &'a [Value]) -> Result<Vec<(&'a str, Params<'a>)>> {
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
    let checked = entries
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            let params = Params::new(entry.as_object().ok_or_else(|| invalid_entry_id(index))?);
            match params.identifier("Id", MAX_ENTRY_ID_CHARS) {
                Ok(Some(id)) => Ok((id, params)),
                _ => Err(invalid_entry_id(index)),
            }
        })
        .collect::<Result<Vec<_>>>()?;
    let mut seen = HashSet::new();
    match checked.iter().find(|(id, _)| !seen.insert(*id)) {
        Some((id, _)) => Err(Error::new(
            ErrorCode::BatchEntryIdsNotDistinct,
            format!("entry `Id` `{id}` is used twice"),
        )),
        None => Ok(checked),
    }
}

impl Broker {
    pub(super) fn send_message_batch(&mut self, params: Params<'_>) -> Result<Members> {
        self.batch(params, Self::send_to)
    }

    pub(super) fn delete_message_batch(&mut self, params: Params<'_>) -> Result<Members> {
        self.batch(params, Self::delete_from)
    }

    fn batch(&mut self, params: Params<'_>, handle_entry: EntryHandler) -> Result<Members> {
        params.require(&["QueueName", "Entries"])?;
        let name = params.required_string("QueueName")?;
        let entries = params.required_array("Entries")?;
        self.queue_mut(name)?;
        let entries = checked_entries(entries)?;

        let mut successful = Vec::new();
        let mut failed = Vec::new();
        for (id, entry) in entries {
            match handle_entry(self, name, entry) {
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
