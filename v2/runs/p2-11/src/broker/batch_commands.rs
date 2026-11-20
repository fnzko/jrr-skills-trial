//! SendMessageBatch and DeleteMessageBatch (feature spec section 2).

use std::collections::HashSet;

use serde_json::{Map, Value};

use super::message_commands::{SendEntry, receipt_handle_param};
use super::{Broker, Members, members};
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
    let mut parsed = Vec::with_capacity(entries.len());
    for entry in entries {
        let invalid_id = || {
            batch_error(
                ErrorCode::InvalidBatchEntryId,
                "every entry needs an `Id` of 1 to 80 characters from A-Z a-z 0-9 _ -",
            )
        };
        let params = entry.as_object().map(Params::new).ok_or_else(invalid_id)?;
        let id = match params.identifier("Id", MAX_ENTRY_ID_CHARS) {
            Ok(Some(id)) => id,
            Ok(None) | Err(_) => return Err(invalid_id()),
        };
        parsed.push((id, params));
    }
    let mut seen = HashSet::new();
    if parsed.iter().any(|(id, _)| !seen.insert(*id)) {
        return Err(batch_error(
            ErrorCode::BatchEntryIdsNotDistinct,
            "two entries have the same `Id`",
        ));
    }
    Ok(parsed)
}

impl Broker {
    pub(super) fn send_message_batch(&mut self, params: Params<'_>) -> Result<Members> {
        self.run_batch(params, |broker, queue, entry| {
            let entry = SendEntry::parse(entry)?;
            broker.send_to_queue(queue, entry)
        })
    }

    pub(super) fn delete_message_batch(&mut self, params: Params<'_>) -> Result<Members> {
        self.run_batch(params, |broker, queue, entry| {
            let handle = receipt_handle_param(entry)?;
            broker.delete_from_queue(queue, handle)
        })
    }

    /// Validates the batch, then runs `handle_entry` for each entry in order.
    /// An entry that fails is reported in `Failed` and does not stop the rest.
    fn run_batch(
        &mut self,
        params: Params<'_>,
        handle_entry: impl Fn(&mut Self, &str, Params<'_>) -> Result<Members>,
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
                Ok(result) => {
                    let mut object = Map::from_iter([("Id".to_owned(), Value::from(id))]);
                    object.extend(result);
                    successful.push(Value::Object(object));
                }
                Err(error) => {
                    failed.push(serde_json::json!({ "Id": id, "error": error.to_json() }))
                }
            }
        }
        Ok(members([
            ("Successful", Value::Array(successful)),
            ("Failed", Value::Array(failed)),
        ]))
    }
}
