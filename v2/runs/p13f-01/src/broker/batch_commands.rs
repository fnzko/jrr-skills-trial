//! Batch message commands (SPEC-feature section 2).

use std::collections::HashSet;

use serde_json::{Value, json};

use super::message_commands::{SendRequest, delete_from_queue, send_to_queue};
use super::{Broker, Members, lookup, members};
use crate::error::{Error, ErrorCode, Result};
use crate::naming;
use crate::request::Params;

const MAX_ENTRIES: usize = 10;
const MAX_ENTRY_ID_CHARS: usize = 80;

/// A batch entry with a valid `Id`.
struct Entry<'a> {
    id: &'a str,
    params: Params<'a>,
}

/// Applies the batch rules 1 to 4 to the `Entries` array.
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
            "`Entries` has more than 10 entries",
        ));
    }
    let parsed = entries
        .iter()
        .map(|entry| {
            let object = entry.as_object();
            let id = object
                .and_then(|o| o.get("Id"))
                .and_then(Value::as_str)
                .filter(|id| naming::is_identifier(id, MAX_ENTRY_ID_CHARS));
            match (object, id) {
                (Some(object), Some(id)) => Ok(Entry {
                    id,
                    params: Params::new(object),
                }),
                _ => Err(Error::new(
                    ErrorCode::InvalidBatchEntryId,
                    "every entry needs an `Id` of 1 to 80 characters from A-Z a-z 0-9 _ -",
                )),
            }
        })
        .collect::<Result<Vec<_>>>()?;
    let mut seen = HashSet::new();
    if parsed.iter().any(|entry| !seen.insert(entry.id)) {
        return Err(Error::new(
            ErrorCode::BatchEntryIdsNotDistinct,
            "entry `Id`s must be distinct",
        ));
    }
    Ok(parsed)
}

/// Handles the entries in order and builds `Successful` and `Failed`.
fn run_entries(
    entries: &[Entry<'_>],
    mut handle: impl FnMut(Params<'_>) -> Result<Members>,
) -> Members {
    let mut successful = Vec::new();
    let mut failed = Vec::new();
    for entry in entries {
        match handle(entry.params) {
            Ok(mut result) => {
                result.insert("Id".into(), json!(entry.id));
                successful.push(Value::Object(result));
            }
            Err(error) => failed.push(json!({ "Id": entry.id, "error": error.to_json() })),
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
        let name = params
            .string("QueueName")?
            .expect("checked by require above");
        let entries = params.array("Entries")?.expect("checked by require above");
        let now = self.now;
        let queue = lookup(&mut self.queues, now, name)?;
        let entries = parse_entries(entries)?;
        let next_message_id = &mut self.next_message_id;
        Ok(run_entries(&entries, |entry| {
            entry.require(&["Body"])?;
            let request = SendRequest::parse(entry)?;
            send_to_queue(queue, next_message_id, now, &request)
        }))
    }

    pub(super) fn delete_message_batch(&mut self, params: Params<'_>) -> Result<Members> {
        params.require(&["QueueName", "Entries"])?;
        let name = params.string("QueueName")?.expect("required");
        let entries = params.array("Entries")?.expect("required");
        let now = self.now;
        let issued = self.next_receipt_handle;
        let queue = lookup(&mut self.queues, now, name)?;
        let entries = parse_entries(entries)?;
        Ok(run_entries(&entries, |entry| {
            entry.require(&["ReceiptHandle"])?;
            let handle = entry
                .string("ReceiptHandle")?
                .expect("checked by require above");
            delete_from_queue(queue, issued, now, handle)
        }))
    }
}
