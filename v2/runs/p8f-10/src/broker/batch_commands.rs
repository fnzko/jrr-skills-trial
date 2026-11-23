//! Batch commands (SPEC-feature section 2).

use std::collections::HashSet;

use serde_json::{Value, json};

use super::message_commands::{SendArgs, missing};
use super::{Broker, Members, lookup, members};
use crate::error::{Error, ErrorCode, Result};
use crate::naming;
use crate::request::Params;

const MAX_ENTRIES: usize = 10;
const MAX_ENTRY_ID_CHARS: usize = 80;

/// One entry of a batch: its `Id` and its other parameters.
struct Entry<'a> {
    id: &'a str,
    params: Params<'a>,
}

fn invalid_entry_id(index: usize) -> Error {
    Error::new(
        ErrorCode::InvalidBatchEntryId,
        format!("entry {index} is not an object with a valid `Id`"),
    )
}

/// Applies the numbered rules 1 to 4 of a batch command to `Entries`.
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
        .enumerate()
        .map(|(index, entry)| {
            let object = entry.as_object().ok_or_else(|| invalid_entry_id(index))?;
            let params = Params::new(object);
            // A non-string `Id` is an invalid entry id by rule 3, not a type error.
            let id = params
                .string("Id")
                .ok()
                .flatten()
                .filter(|id| naming::is_identifier(id, MAX_ENTRY_ID_CHARS))
                .ok_or_else(|| invalid_entry_id(index))?;
            Ok(Entry { id, params })
        })
        .collect::<Result<Vec<_>>>()?;
    let mut seen = HashSet::new();
    if let Some(entry) = parsed.iter().find(|entry| !seen.insert(entry.id)) {
        return Err(Error::new(
            ErrorCode::BatchEntryIdsNotDistinct,
            format!("`Id` `{}` is used twice", entry.id),
        ));
    }
    Ok(parsed)
}

/// Collects per-entry outcomes into the `Successful` and `Failed` members.
fn outcome(results: Vec<(&str, Result<Members>)>) -> Members {
    let mut successful = Vec::new();
    let mut failed = Vec::new();
    for (id, result) in results {
        match result {
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

        let results = entries
            .into_iter()
            .map(|Entry { id, params }| {
                let result = SendArgs::parse(params).and_then(|args| self.send_parsed(name, args));
                (id, result)
            })
            .collect();
        Ok(outcome(results))
    }

    pub(super) fn delete_message_batch(&mut self, params: Params<'_>) -> Result<Members> {
        params.require(&["QueueName", "Entries"])?;
        let name = params.string("QueueName")?.expect("required");
        let entries = params.array("Entries")?.expect("required");
        lookup(&mut self.queues, self.now, name)?;
        let entries = parse_entries(entries)?;

        let results = entries
            .into_iter()
            .map(|Entry { id, params }| {
                let result = params
                    .string("ReceiptHandle")
                    .and_then(|handle| handle.ok_or_else(|| missing("ReceiptHandle")))
                    .and_then(|handle| self.delete_parsed(name, handle));
                (id, result)
            })
            .collect();
        Ok(outcome(results))
    }
}
