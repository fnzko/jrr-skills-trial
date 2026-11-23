//! SendMessageBatch and DeleteMessageBatch (SPEC-feature section 2).

use std::collections::HashSet;

use serde_json::{Value, json};

use super::message_commands::{SendArgs, missing};
use super::{Broker, Members, members};
use crate::error::{Error, ErrorCode, Result};
use crate::request::Params;

const MAX_ENTRIES: usize = 10;
const MAX_ENTRY_ID_CHARS: usize = 80;

/// Applies numbered rules 1 to 4 and returns the entries with their ids.
fn parse_entries<'a>(entries: &'a [Value]) -> Result<Vec<(&'a str, Params<'a>)>> {
    if entries.is_empty() {
        return Err(Error::new(
            ErrorCode::EmptyBatchRequest,
            "`Entries` has no entry",
        ));
    }
    if entries.len() > MAX_ENTRIES {
        return Err(Error::new(
            ErrorCode::TooManyEntriesInBatchRequest,
            "`Entries` has more than 10 entries",
        ));
    }
    let parsed = entries
        .iter()
        .map(|entry| {
            let entry = entry.as_object().map(Params::new);
            let id = entry.and_then(|p| p.identifier("Id", MAX_ENTRY_ID_CHARS).ok().flatten());
            match (id, entry) {
                (Some(id), Some(entry)) => Ok((id, entry)),
                _ => Err(Error::new(
                    ErrorCode::InvalidBatchEntryId,
                    "every entry needs an `Id` of 1 to 80 characters from A-Z a-z 0-9 _ -",
                )),
            }
        })
        .collect::<Result<Vec<_>>>()?;
    let mut seen = HashSet::with_capacity(parsed.len());
    if parsed.iter().any(|(id, _)| !seen.insert(*id)) {
        return Err(Error::new(
            ErrorCode::BatchEntryIdsNotDistinct,
            "entry ids must be distinct",
        ));
    }
    Ok(parsed)
}

/// Collects the per-entry outcomes into the `Successful` and `Failed` members.
fn batch_result(outcomes: impl Iterator<Item = (String, Result<Members>)>) -> Members {
    let mut successful = Vec::new();
    let mut failed = Vec::new();
    for (id, outcome) in outcomes {
        match outcome {
            Ok(mut result) => {
                result.insert("Id".into(), Value::String(id));
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
        let entries = parse_entries(entries)?;

        let outcomes = entries.into_iter().map(|(id, entry)| {
            let outcome = SendArgs::parse(entry).and_then(|args| self.send_to_queue(name, args));
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
                .string("ReceiptHandle")
                .and_then(|handle| handle.ok_or_else(|| missing("ReceiptHandle")))
                .and_then(|handle| self.delete_from_queue(name, handle));
            (id.to_owned(), outcome)
        });
        Ok(batch_result(outcomes))
    }
}
