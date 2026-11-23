//! Batch commands (SPEC-feature section 2).

use std::collections::HashSet;

use serde_json::{Value, json};

use super::message_commands::{Send, delete_from, send_to};
use super::{Broker, Members, lookup, members};
use crate::error::{Error, ErrorCode, Result};
use crate::request::Params;

const MAX_ENTRIES: usize = 10;
const MAX_ID_CHARS: usize = 80;

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
    let mut parsed = Vec::with_capacity(entries.len());
    let mut seen = HashSet::new();
    for entry in entries {
        let invalid = || {
            Error::new(
                ErrorCode::InvalidBatchEntryId,
                "an entry must be an object with an `Id` of 1 to 80 characters from A-Z a-z 0-9 _ -",
            )
        };
        let params = entry.as_object().map(Params::new).ok_or_else(invalid)?;
        let Ok(Some(id)) = params.identifier("Id", MAX_ID_CHARS) else {
            return Err(invalid());
        };
        parsed.push((id, params));
    }
    for &(id, _) in &parsed {
        if !seen.insert(id) {
            return Err(Error::new(
                ErrorCode::BatchEntryIdsNotDistinct,
                format!("entry `Id` `{id}` is used twice"),
            ));
        }
    }
    Ok(parsed)
}

/// Sorts the per-entry results into `Successful` and `Failed`.
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

        let now = self.now;
        let queue = lookup(&mut self.queues, now, name)?;
        let outcomes = parse_entries(entries)?
            .into_iter()
            .map(|(id, entry)| {
                let outcome = entry
                    .require(&["Body"])
                    .and_then(|()| Send::parse(entry))
                    .and_then(|send| send_to(queue, &mut self.next_message_id, now, &send));
                (id, outcome)
            })
            .collect();
        Ok(batch_result(outcomes))
    }

    pub(super) fn delete_message_batch(&mut self, params: Params<'_>) -> Result<Members> {
        params.require(&["QueueName", "Entries"])?;
        let name = params.string("QueueName")?.expect("required");
        let entries = params.array("Entries")?.expect("required");

        let now = self.now;
        let issued = self.next_receipt_handle;
        let queue = lookup(&mut self.queues, now, name)?;
        let outcomes = parse_entries(entries)?
            .into_iter()
            .map(|(id, entry)| {
                let outcome = entry
                    .require(&["ReceiptHandle"])
                    .and_then(|()| {
                        let handle = entry.string("ReceiptHandle")?.expect("required");
                        delete_from(queue, issued, now, handle)
                    })
                    .map(|()| Members::new());
                (id, outcome)
            })
            .collect();
        Ok(batch_result(outcomes))
    }
}
