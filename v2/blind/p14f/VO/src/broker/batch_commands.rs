//! Batch commands (SPEC-feature section 2).

use std::collections::HashSet;

use serde_json::Value;

use super::message_commands::SendEntry;
use super::{Broker, Members, members};
use crate::error::{Error, ErrorCode, Result};
use crate::naming;
use crate::request::Params;

const MAX_ENTRIES: usize = 10;
const MAX_ENTRY_ID_CHARS: usize = 80;

fn invalid_id(message: &str) -> Error {
    Error::new(ErrorCode::InvalidBatchEntryId, message)
}

/// Checks rules 1 to 4 of the batch commands and returns each entry with
/// its `Id`.
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
                .ok_or_else(|| invalid_id("an entry is not an object"))?;
            let entry = Params::new(members);
            let id = entry
                .string("Id")
                .map_err(|_| invalid_id("an entry `Id` is not a string"))?
                .filter(|id| naming::is_identifier(id, MAX_ENTRY_ID_CHARS))
                .ok_or_else(|| invalid_id("an entry has no valid `Id`"))?;
            Ok((id, entry))
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

impl Broker {
    pub(super) fn send_message_batch(&mut self, params: Params<'_>) -> Result<Members> {
        self.run_batch(params, |broker, queue, entry| {
            entry.require(&["Body"])?;
            broker.send_entry(queue, &SendEntry::parse(entry)?)
        })
    }

    pub(super) fn delete_message_batch(&mut self, params: Params<'_>) -> Result<Members> {
        self.run_batch(params, |broker, queue, entry| {
            entry.require(&["ReceiptHandle"])?;
            let handle = entry.required_string("ReceiptHandle")?;
            broker.delete_entry(queue, handle)
        })
    }

    /// Runs `handle` on every entry in order. An entry that fails is
    /// reported in `Failed` and does not stop the others.
    fn run_batch(
        &mut self,
        params: Params<'_>,
        handle: impl Fn(&mut Broker, &str, Params<'_>) -> Result<Members>,
    ) -> Result<Members> {
        params.require(&["QueueName", "Entries"])?;
        let name = params.required_string("QueueName")?;
        let entries = params.required_array("Entries")?;
        self.queue_mut(name)?;
        let entries = parse_entries(entries)?;

        let mut successful = Vec::new();
        let mut failed = Vec::new();
        for (id, entry) in entries {
            match handle(self, name, entry) {
                Ok(mut result) => {
                    result.insert("Id".into(), Value::from(id));
                    successful.push(Value::Object(result));
                }
                Err(error) => failed.push(serde_json::json!({"Id": id, "error": error.to_json()})),
            }
        }
        Ok(members([
            ("Successful", Value::Array(successful)),
            ("Failed", Value::Array(failed)),
        ]))
    }
}
