//! Batch commands (SPEC-feature section 2).

use std::collections::HashSet;

use serde_json::{Map, Value, json};

use super::message_commands::SendParams;
use super::{Broker, Members, lookup, members};
use crate::error::{Error, ErrorCode, Result};
use crate::naming;
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

/// An entry's parameters together with its `Id`.
struct Entry<'a> {
    id: &'a str,
    params: Params<'a>,
}

/// Applies the numbered rules shared by both batch commands.
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
            let members = entry.as_object().ok_or_else(invalid_entry_id)?;
            let params = Params::new(members);
            let id = params
                .string("Id")
                .ok()
                .flatten()
                .filter(|id| naming::is_identifier(id, MAX_ENTRY_ID_CHARS))
                .ok_or_else(invalid_entry_id)?;
            Ok(Entry { id, params })
        })
        .collect::<Result<Vec<_>>>()?;
    let mut seen = HashSet::new();
    if let Some(entry) = parsed.iter().find(|entry| !seen.insert(entry.id)) {
        return Err(Error::new(
            ErrorCode::BatchEntryIdsNotDistinct,
            format!("`Id` `{}` is used by more than one entry", entry.id),
        ));
    }
    Ok(parsed)
}

/// Collects the per-entry outcomes into the `Successful` and `Failed` members.
fn outcomes<'a>(results: impl Iterator<Item = (&'a str, Result<Members>)>) -> Members {
    let mut successful = Vec::new();
    let mut failed = Vec::new();
    for (id, result) in results {
        match result {
            Ok(mut result) => {
                let mut entry = Map::new();
                entry.insert("Id".into(), json!(id));
                entry.append(&mut result);
                successful.push(Value::Object(entry));
            }
            Err(Error { code, message }) => failed.push(json!({
                "Id": id,
                "error": { "code": code.as_str(), "message": message },
            })),
        }
    }
    members([
        ("Successful", Value::Array(successful)),
        ("Failed", Value::Array(failed)),
    ])
}

impl Broker {
    pub(super) fn send_message_batch(&mut self, params: Params<'_>) -> Result<Members> {
        params.require(&["QueueName", "Entries"])?;
        let name = params.string("QueueName")?.expect("required");
        let entries = params.array("Entries")?.expect("required");
        lookup(&mut self.queues, self.now, name)?;
        let entries = parse_entries(entries)?;

        let results = entries.iter().map(|entry| {
            let result = entry
                .params
                .require(&["Body"])
                .and_then(|()| SendParams::parse(entry.params))
                .and_then(|send| self.send_to(name, send));
            (entry.id, result)
        });
        // Entries must see the effects of the ones before them, so the lazy
        // iterator is drained in order here.
        Ok(outcomes(results))
    }

    pub(super) fn delete_message_batch(&mut self, params: Params<'_>) -> Result<Members> {
        params.require(&["QueueName", "Entries"])?;
        let name = params.string("QueueName")?.expect("required");
        let entries = params.array("Entries")?.expect("required");
        lookup(&mut self.queues, self.now, name)?;
        let entries = parse_entries(entries)?;

        let results = entries.iter().map(|entry| {
            let result = entry
                .params
                .require(&["ReceiptHandle"])
                .and_then(|()| entry.params.string("ReceiptHandle"))
                .and_then(|handle| self.delete_from(name, handle.expect("required")));
            (entry.id, result)
        });
        Ok(outcomes(results))
    }
}
