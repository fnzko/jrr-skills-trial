//! `SendMessageBatch` and `DeleteMessageBatch` (SPEC-feature section 2).

use std::collections::HashSet;

use serde_json::{Map, Value, json};

use super::{Broker, Members, members};
use crate::error::{Error, ErrorCode, Result};
use crate::naming;
use crate::request::Params;

const MAX_ENTRIES: usize = 10;
const MAX_ENTRY_ID_CHARS: usize = 80;

/// The checked entries of a batch request: each entry's `Id` and members.
type Entries<'a> = Vec<(&'a str, &'a Map<String, Value>)>;

fn entry_error(code: ErrorCode, message: &str) -> Error {
    Error::new(code, message)
}

/// Numbered rules 1 to 4 of both batch commands.
fn check_entries(entries: &[Value]) -> Result<Entries<'_>> {
    if entries.is_empty() {
        return Err(entry_error(
            ErrorCode::EmptyBatchRequest,
            "`Entries` has no entry",
        ));
    }
    if entries.len() > MAX_ENTRIES {
        return Err(entry_error(
            ErrorCode::TooManyEntriesInBatchRequest,
            "`Entries` has more than 10 entries",
        ));
    }
    let mut checked = Vec::with_capacity(entries.len());
    for entry in entries {
        let id = entry
            .as_object()
            .and_then(|o| Some((o.get("Id")?.as_str()?, o)))
            .filter(|(id, _)| naming::is_identifier(id, MAX_ENTRY_ID_CHARS));
        match id {
            Some(pair) => checked.push(pair),
            None => {
                return Err(entry_error(
                    ErrorCode::InvalidBatchEntryId,
                    "an entry is not an object or has no valid `Id`",
                ));
            }
        }
    }
    let mut seen = HashSet::new();
    if !checked.iter().all(|(id, _)| seen.insert(*id)) {
        return Err(entry_error(
            ErrorCode::BatchEntryIdsNotDistinct,
            "two entries have the same `Id`",
        ));
    }
    Ok(checked)
}

impl Broker {
    pub(super) fn send_message_batch(&mut self, params: Params<'_>) -> Result<Members> {
        self.batch(params, Self::send_message)
    }

    pub(super) fn delete_message_batch(&mut self, params: Params<'_>) -> Result<Members> {
        self.batch(params, Self::delete_message)
    }

    /// Runs `single` for each entry as if it were a command with the same
    /// `QueueName` and the entry's members.
    fn batch(
        &mut self,
        params: Params<'_>,
        single: fn(&mut Self, Params<'_>) -> Result<Members>,
    ) -> Result<Members> {
        params.require(&["QueueName", "Entries"])?;
        let name = params.string("QueueName")?.expect("required");
        let entries = params.array("Entries")?.expect("required");
        self.queue_mut(name)?;
        let entries = check_entries(entries)?;

        let mut successful = Vec::new();
        let mut failed = Vec::new();
        for (id, entry) in entries {
            let mut command = entry.clone();
            command.insert("QueueName".into(), json!(name));
            match single(self, Params::new(&command)) {
                Ok(mut result) => {
                    result.insert("Id".into(), json!(id));
                    successful.push(Value::Object(result));
                }
                Err(error) => failed.push(json!({
                    "Id": id,
                    "error": { "code": error.code.as_str(), "message": error.message },
                })),
            }
        }
        Ok(members([
            ("Successful", Value::Array(successful)),
            ("Failed", Value::Array(failed)),
        ]))
    }
}
