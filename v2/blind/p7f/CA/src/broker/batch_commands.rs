//! Batch commands (SPEC-feature section 2).

use std::collections::HashSet;

use serde_json::{Map, Value};

use super::{Broker, Members};
use crate::error::{Error, ErrorCode, Result};
use crate::request::Params;

const MAX_ENTRIES: usize = 10;
const MAX_ENTRY_ID_CHARS: usize = 80;

fn batch_error(code: ErrorCode, message: &str) -> Error {
    Error::new(code, message)
}

/// Checks rules 1 to 4 and returns each entry with its `Id`.
fn parse_entries(entries: &[Value]) -> Result<Vec<(&str, Params<'_>)>> {
    if entries.is_empty() {
        return Err(batch_error(
            ErrorCode::EmptyBatchRequest,
            "`Entries` has no entry",
        ));
    }
    if entries.len() > MAX_ENTRIES {
        return Err(batch_error(
            ErrorCode::TooManyEntriesInBatchRequest,
            "`Entries` has more than 10 entries",
        ));
    }
    let parsed = entries
        .iter()
        .map(|entry| {
            let params = entry.as_object().map(Params::new);
            let id = params.and_then(|p| p.identifier("Id", MAX_ENTRY_ID_CHARS).ok().flatten());
            params
                .zip(id)
                .map(|(params, id)| (id, params))
                .ok_or_else(|| {
                    batch_error(
                        ErrorCode::InvalidBatchEntryId,
                        "every entry must be an object with a valid `Id`",
                    )
                })
        })
        .collect::<Result<Vec<_>>>()?;
    let mut seen = HashSet::new();
    if parsed.iter().any(|(id, _)| !seen.insert(*id)) {
        return Err(batch_error(
            ErrorCode::BatchEntryIdsNotDistinct,
            "two entries have the same `Id`",
        ));
    }
    Ok(parsed)
}

/// Builds the `Successful` and `Failed` members from per-entry outcomes.
fn batch_result(outcomes: impl Iterator<Item = (String, Result<Members>)>) -> Members {
    let mut successful = Vec::new();
    let mut failed = Vec::new();
    for (id, outcome) in outcomes {
        match outcome {
            Ok(mut result) => {
                result.insert("Id".into(), Value::String(id));
                successful.push(Value::Object(result));
            }
            Err(error) => {
                let mut entry = Map::new();
                entry.insert("Id".into(), Value::String(id));
                entry.insert("error".into(), error.to_json());
                failed.push(Value::Object(entry));
            }
        }
    }
    let mut result = Members::new();
    result.insert("Successful".into(), Value::Array(successful));
    result.insert("Failed".into(), Value::Array(failed));
    result
}

impl Broker {
    pub(super) fn send_message_batch(&mut self, params: Params<'_>) -> Result<Members> {
        params.require(&["QueueName", "Entries"])?;
        let name = params.string("QueueName")?.expect("required");
        let entries = params.array("Entries")?.expect("required");
        self.queue_mut(name)?;
        let entries = parse_entries(entries)?;
        let outcomes = entries.into_iter().map(|(id, entry)| {
            let outcome = entry
                .require(&["Body"])
                .and_then(|()| self.send_to_queue(name, entry));
            (id.to_owned(), outcome)
        });
        Ok(batch_result(outcomes))
    }

    pub(super) fn delete_message_batch(&mut self, params: Params<'_>) -> Result<Members> {
        params.require(&["QueueName", "Entries"])?;
        let name = params.string("QueueName")?.expect("required");
        let entries = params.array("Entries")?.expect("required");
        self.queue_mut(name)?;
        let entries = parse_entries(entries)?;
        let outcomes = entries.into_iter().map(|(id, entry)| {
            let outcome = entry
                .require(&["ReceiptHandle"])
                .and_then(|()| self.delete_from_queue(name, entry));
            (id.to_owned(), outcome)
        });
        Ok(batch_result(outcomes))
    }
}
