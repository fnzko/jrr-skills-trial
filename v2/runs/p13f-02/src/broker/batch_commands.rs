//! Batch commands (SPEC-feature section 2).

use std::collections::HashSet;

use serde_json::{Map, Value, json};

use super::{Broker, Members, error_json};
use crate::error::{Error, ErrorCode, Result};
use crate::request::Params;

const MAX_ENTRIES: usize = 10;
const MAX_ENTRY_ID_CHARS: usize = 80;

/// Handles one entry of a batch against the queue named by its first argument.
type EntryHandler = fn(&mut Broker, &str, Params<'_>) -> Result<Members>;

fn invalid_entry_id(message: &str) -> Error {
    Error::new(ErrorCode::InvalidBatchEntryId, message)
}

/// The members of an entry and its `Id`, once the batch rules 3 and 4 hold.
fn entry_members(entry: &Value) -> Result<(&Map<String, Value>, &str)> {
    let members = entry
        .as_object()
        .ok_or_else(|| invalid_entry_id("an entry must be an object"))?;
    let id = Params::new(members)
        .identifier("Id", MAX_ENTRY_ID_CHARS)
        .map_err(|e| invalid_entry_id(&e.message))?
        .ok_or_else(|| invalid_entry_id("an entry needs an `Id`"))?;
    Ok((members, id))
}

impl Broker {
    pub(super) fn send_message_batch(&mut self, params: Params<'_>) -> Result<Members> {
        self.run_batch(params, |broker, name, entry| {
            entry.require(&["Body"])?;
            broker.send_entry(name, entry)
        })
    }

    pub(super) fn delete_message_batch(&mut self, params: Params<'_>) -> Result<Members> {
        self.run_batch(params, |broker, name, entry| {
            entry.require(&["ReceiptHandle"])?;
            broker.delete_entry(name, entry)
        })
    }

    fn run_batch(&mut self, params: Params<'_>, handle: EntryHandler) -> Result<Members> {
        params.require(&["QueueName", "Entries"])?;
        let name = params
            .string("QueueName")?
            .expect("present: require() checked it above");
        let entries = params
            .array("Entries")?
            .expect("present: require() checked it above");
        self.queue_mut(name)?;

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
        let entries = entries
            .iter()
            .map(entry_members)
            .collect::<Result<Vec<_>>>()?;
        let mut seen = HashSet::new();
        if let Some((_, id)) = entries.iter().find(|(_, id)| !seen.insert(*id)) {
            return Err(Error::new(
                ErrorCode::BatchEntryIdsNotDistinct,
                format!("entry `Id` `{id}` is used twice"),
            ));
        }

        let mut successful = Vec::new();
        let mut failed = Vec::new();
        for (members, id) in entries {
            match handle(self, name, Params::new(members)) {
                Ok(mut result) => {
                    result.insert("Id".into(), json!(id));
                    successful.push(Value::Object(result));
                }
                Err(error) => failed.push(json!({ "Id": id, "error": error_json(&error) })),
            }
        }
        Ok(super::members([
            ("Successful", Value::Array(successful)),
            ("Failed", Value::Array(failed)),
        ]))
    }
}
