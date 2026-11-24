//! SendMessageBatch and DeleteMessageBatch (SPEC-feature section 2).

use std::collections::HashSet;

use serde_json::{Value, json};

use super::{Broker, Members, members};
use crate::error::{Error, ErrorCode, Result};
use crate::request::Params;

const MAX_ENTRIES: usize = 10;
const MAX_ENTRY_ID_CHARS: usize = 80;

/// Applies one entry to a queue that exists, as the single command would.
type EntryHandler = fn(&mut Broker, &str, Params<'_>) -> Result<Members>;

fn invalid_entry_id(index: usize) -> Error {
    Error::new(
        ErrorCode::InvalidBatchEntryId,
        format!(
            "entry {index} must be an object with an `Id` of 1 to {MAX_ENTRY_ID_CHARS} characters from A-Z a-z 0-9 _ -"
        ),
    )
}

/// Checks rules 1 to 4 and returns each entry's `Id` with its parameters.
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
            format!("`Entries` has more than {MAX_ENTRIES} entries"),
        ));
    }
    let parsed = entries
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            let entry = entry
                .as_object()
                .map(Params::new)
                .ok_or_else(|| invalid_entry_id(index))?;
            let id = entry
                .identifier("Id", MAX_ENTRY_ID_CHARS)
                .ok()
                .flatten()
                .ok_or_else(|| invalid_entry_id(index))?;
            Ok((id, entry))
        })
        .collect::<Result<Vec<_>>>()?;
    let mut seen = HashSet::new();
    match parsed.iter().find(|(id, _)| !seen.insert(*id)) {
        Some((id, _)) => Err(Error::new(
            ErrorCode::BatchEntryIdsNotDistinct,
            format!("entry `Id` `{id}` is used twice"),
        )),
        None => Ok(parsed),
    }
}

impl Broker {
    pub(super) fn send_message_batch(&mut self, params: Params<'_>) -> Result<Members> {
        self.run_batch(params, Broker::send_to)
    }

    pub(super) fn delete_message_batch(&mut self, params: Params<'_>) -> Result<Members> {
        self.run_batch(params, Broker::delete_from)
    }

    fn run_batch(&mut self, params: Params<'_>, handle: EntryHandler) -> Result<Members> {
        params.require(&["QueueName", "Entries"])?;
        let name = params.required_string("QueueName")?;
        let entries = params.required_array("Entries")?;
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
                Err(Error { code, message }) => failed.push(json!({
                    "Id": id,
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
