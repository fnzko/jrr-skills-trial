//! Batch commands (SPEC-feature section 2).

use std::collections::HashSet;

use serde_json::{Value, json};

use super::message_commands::SendArgs;
use super::{Broker, Members, members};
use crate::error::{Error, ErrorCode, Result};
use crate::request::Params;

const MAX_ENTRIES: usize = 10;
const MAX_ENTRY_ID_CHARS: usize = 80;

fn invalid_entry_id(message: &str) -> Error {
    Error::new(ErrorCode::InvalidBatchEntryId, message)
}

/// The `Entries` of a batch request, checked by numbered rules 1 to 4.
/// Each element is an object whose `Id` is valid and distinct.
fn batch_entries<'a>(entries: &'a [Value]) -> Result<Vec<(&'a str, Params<'a>)>> {
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
            let object = entry
                .as_object()
                .ok_or_else(|| invalid_entry_id("a batch entry is not an object"))?;
            let params = Params::new(object);
            let id = params
                .identifier("Id", MAX_ENTRY_ID_CHARS)
                .ok()
                .flatten()
                .ok_or_else(|| invalid_entry_id("a batch entry has no valid `Id`"))?;
            Ok((id, params))
        })
        .collect::<Result<Vec<_>>>()?;
    let mut seen = HashSet::new();
    if let Some((id, _)) = parsed.iter().find(|(id, _)| !seen.insert(*id)) {
        return Err(Error::new(
            ErrorCode::BatchEntryIdsNotDistinct,
            format!("batch entry id `{id}` is used twice"),
        ));
    }
    Ok(parsed)
}

/// The response members of a batch, from the outcome of each entry in order.
fn batch_result(outcomes: Vec<(&str, Result<Members>)>) -> Members {
    let mut successful = Vec::new();
    let mut failed = Vec::new();
    for (id, outcome) in outcomes {
        match outcome {
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
        self.queue_mut(name)?;

        let outcomes = batch_entries(entries)?
            .into_iter()
            .map(|(id, entry)| {
                let outcome = entry
                    .require(&["Body"])
                    .and_then(|()| SendArgs::parse(entry))
                    .and_then(|args| self.send_to(name, &args));
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

        let outcomes = batch_entries(entries)?
            .into_iter()
            .map(|(id, entry)| {
                let outcome = entry
                    .require(&["ReceiptHandle"])
                    .and_then(|()| Ok(entry.string("ReceiptHandle")?.expect("required")))
                    .and_then(|handle| self.delete_handle(name, handle));
                (id, outcome)
            })
            .collect();
        Ok(batch_result(outcomes))
    }
}
