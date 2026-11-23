//! Batch commands (SPEC-feature section 2).

use std::collections::HashSet;

use serde_json::{Map, Value, json};

use super::message_commands::SendArgs;
use super::{Broker, Members};
use crate::error::{Error, ErrorCode, Result};
use crate::request::Params;

const MAX_ENTRIES: usize = 10;
const MAX_ENTRY_ID_CHARS: usize = 80;

fn invalid_entry_id() -> Error {
    Error::new(
        ErrorCode::InvalidBatchEntryId,
        format!(
            "every entry must be an object with an `Id` of 1 to {MAX_ENTRY_ID_CHARS} characters from A-Z a-z 0-9 _ -"
        ),
    )
}

/// An entry that passed the `Id` rule: its id and its other members.
struct Entry<'a> {
    id: &'a str,
    params: Params<'a>,
}

/// Applies rules 1 to 4 of the batch commands to `Entries`.
fn check_entries(entries: &[Value]) -> Result<Vec<Entry<'_>>> {
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
        .map(|entry| {
            let members: &Map<String, Value> = entry.as_object().ok_or_else(invalid_entry_id)?;
            let params = Params::new(members);
            let id = params
                .identifier("Id", MAX_ENTRY_ID_CHARS)
                .ok()
                .flatten()
                .ok_or_else(invalid_entry_id)?;
            Ok(Entry { id, params })
        })
        .collect::<Result<Vec<_>>>()?;
    let mut seen = HashSet::new();
    if let Some(entry) = entries.iter().find(|entry| !seen.insert(entry.id)) {
        return Err(Error::new(
            ErrorCode::BatchEntryIdsNotDistinct,
            format!("entry id `{}` is used twice", entry.id),
        ));
    }
    Ok(entries)
}

impl Broker {
    pub(super) fn send_message_batch(&mut self, params: Params<'_>) -> Result<Members> {
        self.run_batch(params, |broker, name, entry| {
            entry.require(&["Body"])?;
            let send = SendArgs::parse(entry)?;
            broker.send_to_queue(name, &send)
        })
    }

    pub(super) fn delete_message_batch(&mut self, params: Params<'_>) -> Result<Members> {
        self.run_batch(params, |broker, name, entry| {
            entry.require(&["ReceiptHandle"])?;
            let handle = entry.string("ReceiptHandle")?.expect("required");
            broker.delete_from_queue(name, handle)
        })
    }

    /// Checks the batch as a whole, then runs `handle` for each entry in
    /// order. `handle` does the checks of SPEC section 2 from step 2 on.
    fn run_batch(
        &mut self,
        params: Params<'_>,
        mut handle: impl FnMut(&mut Self, &str, Params<'_>) -> Result<Members>,
    ) -> Result<Members> {
        params.require(&["QueueName", "Entries"])?;
        let name = params.string("QueueName")?.expect("required");
        let entries = params.array("Entries")?.expect("required");
        self.queue_mut(name)?;
        let entries = check_entries(entries)?;

        let mut successful = Vec::new();
        let mut failed = Vec::new();
        for Entry { id, params } in entries {
            match handle(self, name, params) {
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
