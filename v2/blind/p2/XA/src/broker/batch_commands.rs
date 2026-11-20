//! Batch message commands (SPEC-feature section 2).

use std::collections::HashSet;

use serde_json::{Value, json};

use super::message_commands::SendArgs;
use super::{Broker, Members, members};
use crate::error::{Error, ErrorCode, Result};
use crate::request::Params;

const MAX_ENTRIES: usize = 10;
const MAX_ID_CHARS: usize = 80;

fn invalid_entry_id(index: usize) -> Error {
    Error::new(
        ErrorCode::InvalidBatchEntryId,
        format!("entry {index} is not an object with a valid `Id`"),
    )
}

/// Checks the numbered rules 1 to 4 and returns each entry with its `Id`.
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
    let mut parsed = Vec::with_capacity(entries.len());
    for (index, entry) in entries.iter().enumerate() {
        let entry = entry.as_object().ok_or_else(|| invalid_entry_id(index))?;
        let params = Params::new(entry);
        let id = params
            .identifier("Id", MAX_ID_CHARS)
            .ok()
            .flatten()
            .ok_or_else(|| invalid_entry_id(index))?;
        parsed.push((id, params));
    }
    let mut seen = HashSet::new();
    if let Some((id, _)) = parsed.iter().find(|(id, _)| !seen.insert(*id)) {
        return Err(Error::new(
            ErrorCode::BatchEntryIdsNotDistinct,
            format!("entry `Id` `{id}` is used twice"),
        ));
    }
    Ok(parsed)
}

/// Builds the `Successful` and `Failed` result members.
fn batch_result(outcomes: Vec<(&str, Result<Members>)>) -> Members {
    let mut successful = Vec::new();
    let mut failed = Vec::new();
    for (id, outcome) in outcomes {
        match outcome {
            Ok(mut result) => {
                result.insert("Id".into(), json!(id));
                successful.push(Value::Object(result));
            }
            Err(error) => failed.push(json!({ "Id": id, "error": error.to_json() })),
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
        self.queue_mut(name)?;
        let entries = parse_entries(entries)?;

        let outcomes = entries
            .into_iter()
            .map(|(id, entry)| {
                let outcome = entry
                    .require(&["Body"])
                    .and_then(|()| SendArgs::parse(entry))
                    .and_then(|args| self.send_one(name, &args));
                (id, outcome)
            })
            .collect();
        Ok(batch_result(outcomes))
    }

    pub(super) fn delete_message_batch(&mut self, params: Params<'_>) -> Result<Members> {
        params.require(&["QueueName", "Entries"])?;
        let name = params.string("QueueName")?.expect("required");
        let entries = params.array("Entries")?.expect("required");
        self.queue_mut(name)?;
        let entries = parse_entries(entries)?;

        let outcomes = entries
            .into_iter()
            .map(|(id, entry)| {
                let outcome = entry.require(&["ReceiptHandle"]).and_then(|()| {
                    let handle = entry.string("ReceiptHandle")?.expect("required");
                    self.delete_one(name, handle)
                });
                (id, outcome)
            })
            .collect();
        Ok(batch_result(outcomes))
    }
}
