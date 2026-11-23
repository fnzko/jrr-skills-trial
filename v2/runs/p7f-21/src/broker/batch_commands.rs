//! Batch commands (SPEC-feature section 2).

use std::collections::HashSet;

use serde_json::{Map, Value, json};

use super::message_commands::SendArgs;
use super::{Broker, Members, members};
use crate::error::{Error, ErrorCode, Result};
use crate::naming;
use crate::request::Params;

const MAX_ENTRIES: usize = 10;
const MAX_ENTRY_ID_CHARS: usize = 80;

/// A batch entry whose `Id` has passed the batch-level checks.
struct Entry<'a> {
    id: &'a str,
    members: &'a Map<String, Value>,
}

fn invalid_entry_id(message: &str) -> Error {
    Error::new(ErrorCode::InvalidBatchEntryId, message)
}

/// Applies the numbered rules 1 to 4 to `Entries`.
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
    let checked = entries
        .iter()
        .map(|entry| {
            let members = entry
                .as_object()
                .ok_or_else(|| invalid_entry_id("an entry is not an object"))?;
            let id = members
                .get("Id")
                .and_then(Value::as_str)
                .filter(|id| naming::is_identifier(id, MAX_ENTRY_ID_CHARS))
                .ok_or_else(|| invalid_entry_id("an entry has no valid `Id`"))?;
            Ok(Entry { id, members })
        })
        .collect::<Result<Vec<_>>>()?;
    let mut seen = HashSet::new();
    if !checked.iter().all(|entry| seen.insert(entry.id)) {
        return Err(Error::new(
            ErrorCode::BatchEntryIdsNotDistinct,
            "two entries have the same `Id`",
        ));
    }
    Ok(checked)
}

/// Collects per-entry outcomes into the `Successful` and `Failed` members.
#[derive(Default)]
struct Outcomes {
    successful: Vec<Value>,
    failed: Vec<Value>,
}

impl Outcomes {
    fn record(&mut self, id: &str, result: Result<Members>) {
        match result {
            Ok(mut result) => {
                result.insert("Id".into(), json!(id));
                self.successful.push(Value::Object(result));
            }
            Err(Error { code, message }) => self.failed.push(json!({
                "Id": id,
                "error": { "code": code.as_str(), "message": message },
            })),
        }
    }

    fn into_members(self) -> Members {
        members([
            ("Successful", Value::Array(self.successful)),
            ("Failed", Value::Array(self.failed)),
        ])
    }
}

impl Broker {
    /// Shared front half of both batch commands: required and typed
    /// parameters, queue lookup and the numbered rules. Returns the queue
    /// name and the checked entries.
    fn start_batch<'a>(&mut self, params: Params<'a>) -> Result<(&'a str, Vec<Entry<'a>>)> {
        params.require(&["QueueName", "Entries"])?;
        let name = params.string("QueueName")?.expect("required");
        let entries = params.array("Entries")?.expect("required");
        self.queue_mut(name)?;
        Ok((name, check_entries(entries)?))
    }

    pub(super) fn send_message_batch(&mut self, params: Params<'_>) -> Result<Members> {
        let (name, entries) = self.start_batch(params)?;
        let mut outcomes = Outcomes::default();
        for entry in entries {
            let result = SendArgs::parse(Params::new(entry.members))
                .and_then(|args| self.send_to_queue(name, args));
            outcomes.record(entry.id, result);
        }
        Ok(outcomes.into_members())
    }

    pub(super) fn delete_message_batch(&mut self, params: Params<'_>) -> Result<Members> {
        let (name, entries) = self.start_batch(params)?;
        let mut outcomes = Outcomes::default();
        for entry in entries {
            let entry_params = Params::new(entry.members);
            let result = entry_params
                .require(&["ReceiptHandle"])
                .and_then(|()| entry_params.string("ReceiptHandle"))
                .and_then(|handle| self.delete_in_queue(name, handle.expect("required")));
            outcomes.record(entry.id, result);
        }
        Ok(outcomes.into_members())
    }
}
