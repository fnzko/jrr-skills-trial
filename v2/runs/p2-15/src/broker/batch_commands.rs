//! `SendMessageBatch` and `DeleteMessageBatch` (SPEC-feature section 2).

use std::collections::HashSet;

use serde_json::{Value, json};

use super::message_commands::{SendArgs, delete_from_queue, send_to_queue};
use super::{Broker, Members, lookup, members};
use crate::error::{Error, ErrorCode, Result};
use crate::queue::Queue;
use crate::request::Params;

const MAX_ENTRIES: usize = 10;
const MAX_ENTRY_ID_CHARS: usize = 80;

fn invalid_entry_id(message: &str) -> Error {
    Error::new(ErrorCode::InvalidBatchEntryId, message)
}

/// Applies the numbered rules 1 to 4 and returns each entry's `Id` with its
/// parameters, in order.
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
        .map(|entry| {
            let members = entry
                .as_object()
                .ok_or_else(|| invalid_entry_id("an entry is not an object"))?;
            let params = Params::new(members);
            let id = params
                .identifier("Id", MAX_ENTRY_ID_CHARS)
                .ok()
                .flatten()
                .ok_or_else(|| invalid_entry_id("an entry has no valid `Id`"))?;
            Ok((id, params))
        })
        .collect::<Result<Vec<_>>>()?;
    let mut seen = HashSet::new();
    if let Some((id, _)) = parsed.iter().find(|(id, _)| !seen.insert(*id)) {
        return Err(Error::new(
            ErrorCode::BatchEntryIdsNotDistinct,
            format!("entry `Id` `{id}` is used twice"),
        ));
    }
    Ok(parsed)
}

/// Runs `handle` for every entry in order and builds the `Successful` and
/// `Failed` result members.
fn run_entries<'a>(
    entries: Vec<(&'a str, Params<'a>)>,
    mut handle: impl FnMut(Params<'a>) -> Result<Members>,
) -> Members {
    let mut successful = Vec::new();
    let mut failed = Vec::new();
    for (id, params) in entries {
        match handle(params) {
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

/// Steps 2 and 3 of the check order for both batch commands; the caller
/// does the queue lookup and `parse_entries`.
fn batch_params<'a>(params: Params<'a>) -> Result<(&'a str, &'a [Value])> {
    params.require(&["QueueName", "Entries"])?;
    let name = params.string("QueueName")?.expect("required");
    let entries = params.array("Entries")?.expect("required");
    Ok((name, entries))
}

impl Broker {
    pub(super) fn send_message_batch(&mut self, params: Params<'_>) -> Result<Members> {
        let (name, entries) = batch_params(params)?;
        let now = self.now;
        let queue = lookup(&mut self.queues, now, name)?;
        let entries = parse_entries(entries)?;

        let next_message_id = &mut self.next_message_id;
        Ok(run_entries(entries, |entry| {
            let args = SendArgs::parse(entry)?;
            send_to_queue(queue, next_message_id, now, args)
        }))
    }

    pub(super) fn delete_message_batch(&mut self, params: Params<'_>) -> Result<Members> {
        let (name, entries) = batch_params(params)?;
        let (now, issued) = (self.now, self.next_receipt_handle);
        let queue = lookup(&mut self.queues, now, name)?;
        let entries = parse_entries(entries)?;

        Ok(run_entries(entries, |entry| {
            delete_entry(queue, issued, now, entry)
        }))
    }
}

fn delete_entry(queue: &mut Queue, issued: u64, now: u64, entry: Params<'_>) -> Result<Members> {
    entry.require(&["ReceiptHandle"])?;
    let handle = entry.string("ReceiptHandle")?.expect("required");
    delete_from_queue(queue, issued, now, handle)
}
