//! Batch commands: `SendMessageBatch` and `DeleteMessageBatch` (SPEC-feature
//! section 2).

use std::collections::HashSet;

use serde_json::{Value, json};

use super::message_commands::SendArgs;
use super::{Broker, Members, members};
use crate::error::{Error, ErrorCode, Result};
use crate::naming;
use crate::request::Params;

const MAX_ENTRIES: usize = 10;
const MAX_ENTRY_ID_CHARS: usize = 80;

fn invalid_entry_id(message: &str) -> Error {
    Error::new(ErrorCode::InvalidBatchEntryId, message)
}

/// Checks the rules that apply to the batch as a whole and returns each
/// entry's `Id` with its parameters.
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
            let object = entry
                .as_object()
                .ok_or_else(|| invalid_entry_id("an entry is not an object"))?;
            let params = Params::new(object);
            let id = params
                .string("Id")
                .map_err(|_| invalid_entry_id("an entry `Id` is not a string"))?
                .filter(|id| naming::is_identifier(id, MAX_ENTRY_ID_CHARS))
                .ok_or_else(|| invalid_entry_id("an entry has no valid `Id`"))?;
            Ok((id, params))
        })
        .collect::<Result<Vec<_>>>()?;
    let mut seen = HashSet::new();
    if let Some((id, _)) = parsed.iter().find(|(id, _)| !seen.insert(*id)) {
        return Err(Error::new(
            ErrorCode::BatchEntryIdsNotDistinct,
            format!("the entry `Id` `{id}` is used twice"),
        ));
    }
    Ok(parsed)
}

impl Broker {
    pub(super) fn send_message_batch(&mut self, params: Params<'_>) -> Result<Members> {
        self.batch(params, |broker, queue, entry| {
            entry.require(&["Body"])?;
            let args = SendArgs::parse(entry)?;
            broker.send_parsed(queue, &args)
        })
    }

    pub(super) fn delete_message_batch(&mut self, params: Params<'_>) -> Result<Members> {
        self.batch(params, |broker, queue, entry| {
            entry.require(&["ReceiptHandle"])?;
            let handle = entry.string("ReceiptHandle")?.expect("required");
            broker.delete_parsed(queue, handle)
        })
    }

    /// Runs `handle_entry` for each entry in order, as the single command
    /// would run with the same `QueueName`.
    fn batch(
        &mut self,
        params: Params<'_>,
        mut handle_entry: impl FnMut(&mut Self, &str, Params<'_>) -> Result<Members>,
    ) -> Result<Members> {
        params.require(&["QueueName", "Entries"])?;
        let name = params.string("QueueName")?.expect("required");
        let entries = params.array("Entries")?.expect("required");
        self.queue_mut(name)?;
        let entries = parse_entries(entries)?;

        let mut successful = Vec::new();
        let mut failed = Vec::new();
        for (id, entry) in entries {
            match handle_entry(self, name, entry) {
                Ok(mut result) => {
                    result.insert("Id".into(), json!(id));
                    successful.push(Value::Object(result));
                }
                Err(error) => failed.push(json!({ "Id": id, "error": error.to_json() })),
            }
        }
        Ok(members([
            ("Successful", Value::Array(successful)),
            ("Failed", Value::Array(failed)),
        ]))
    }
}
