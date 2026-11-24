//! Batch commands (SPEC-feature section 2).

use serde_json::{Value, json};

use super::message_commands::{SendArgs, accept_send};
use super::{Broker, Members, lookup, members};
use crate::error::{Error, ErrorCode, Result};
use crate::naming;
use crate::request::Params;

const MAX_ENTRIES: usize = 10;
const MAX_ENTRY_ID_CHARS: usize = 80;

fn invalid_entry_id(message: &str) -> Error {
    Error::new(ErrorCode::InvalidBatchEntryId, message)
}

/// Applies the numbered rules 1 to 4 and returns each entry's `Id` with the
/// entry's parameters, in order.
fn batch_entries(entries: &[Value]) -> Result<Vec<(&str, Params<'_>)>> {
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
            // A non-string `Id` is as invalid as a missing one (rule 3).
            let id = params
                .string("Id")
                .ok()
                .flatten()
                .filter(|id| naming::is_identifier(id, MAX_ENTRY_ID_CHARS))
                .ok_or_else(|| {
                    invalid_entry_id(
                        "an entry `Id` must be 1 to 80 characters from A-Z a-z 0-9 _ -",
                    )
                })?;
            Ok((id, params))
        })
        .collect::<Result<Vec<_>>>()?;
    for (i, (id, _)) in parsed.iter().enumerate() {
        if parsed[..i].iter().any(|(earlier, _)| earlier == id) {
            return Err(Error::new(
                ErrorCode::BatchEntryIdsNotDistinct,
                format!("entry `Id` `{id}` is used twice"),
            ));
        }
    }
    Ok(parsed)
}

/// Handles the entries in order and builds the `Successful` and `Failed`
/// result members. A failed entry does not stop the others.
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

        let now = self.now;
        let queue = lookup(&mut self.queues, now, name)?;
        let entries = batch_entries(entries)?;
        let next_message_id = &mut self.next_message_id;
        Ok(run_entries(entries, |entry| {
            entry.require(&["Body"])?;
            let args = SendArgs::parse(entry)?;
            accept_send(queue, next_message_id, now, &args)
        }))
    }

    pub(super) fn delete_message_batch(&mut self, params: Params<'_>) -> Result<Members> {
        params.require(&["QueueName", "Entries"])?;
        let name = params.string("QueueName")?.expect("required");
        let entries = params.array("Entries")?.expect("required");

        self.queue_mut(name)?;
        let entries = batch_entries(entries)?;
        Ok(run_entries(entries, |entry| {
            entry.require(&["ReceiptHandle"])?;
            let handle = entry.string("ReceiptHandle")?.expect("required");
            self.delete_handle(name, handle)?;
            Ok(Members::new())
        }))
    }
}
