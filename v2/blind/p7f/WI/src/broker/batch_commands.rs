//! Batch commands (SPEC-feature section 2).

use std::collections::HashSet;

use serde_json::{Value, json};

use super::{Broker, Members, members};
use crate::error::{Error, ErrorCode, Result};
use crate::request::Params;

const MAX_ENTRIES: usize = 10;
const MAX_ENTRY_ID_CHARS: usize = 80;

/// A batch entry: its `Id` and the object that carries its parameters.
struct Entry<'a> {
    id: &'a str,
    params: Params<'a>,
}

fn batch_error(code: ErrorCode, message: &str) -> Error {
    Error::new(code, message)
}

/// Reads the entry's `Id`, which must be a valid identifier.
fn parse_entry(value: &Value) -> Result<Entry<'_>> {
    let invalid = || batch_error(ErrorCode::InvalidBatchEntryId, "entry has no valid `Id`");
    let params = value.as_object().map(Params::new).ok_or_else(invalid)?;
    let id = params
        .identifier("Id", MAX_ENTRY_ID_CHARS)
        .map_err(|_| invalid())?
        .ok_or_else(invalid)?;
    Ok(Entry { id, params })
}

fn failed_json(id: &str, error: &Error) -> Value {
    json!({
        "Id": id,
        "error": { "code": error.code.as_str(), "message": error.message },
    })
}

impl Broker {
    pub(super) fn send_message_batch(&mut self, params: Params<'_>) -> Result<Members> {
        let (name, entries) = self.batch_entries(params)?;
        self.run_batch(&entries, |broker, entry| {
            entry.params.require(&["Body"])?;
            broker.send_to(name, entry.params)
        })
    }

    pub(super) fn delete_message_batch(&mut self, params: Params<'_>) -> Result<Members> {
        let (name, entries) = self.batch_entries(params)?;
        self.run_batch(&entries, |broker, entry| {
            entry.params.require(&["ReceiptHandle"])?;
            let handle = entry.params.string("ReceiptHandle")?.expect("required");
            broker.delete_handle(name, handle)
        })
    }

    /// Checks the request up to the batch rules and returns the queue name
    /// and the entries.
    fn batch_entries<'a>(&mut self, params: Params<'a>) -> Result<(&'a str, Vec<Entry<'a>>)> {
        params.require(&["QueueName", "Entries"])?;
        let name = params.string("QueueName")?.expect("required");
        let values = params.array("Entries")?.expect("required");
        self.queue_mut(name)?;

        if values.is_empty() {
            return Err(batch_error(
                ErrorCode::EmptyBatchRequest,
                "`Entries` has no entry",
            ));
        }
        if values.len() > MAX_ENTRIES {
            return Err(batch_error(
                ErrorCode::TooManyEntriesInBatchRequest,
                "`Entries` has more than 10 entries",
            ));
        }
        let entries = values.iter().map(parse_entry).collect::<Result<Vec<_>>>()?;
        let mut seen = HashSet::new();
        if let Some(entry) = entries.iter().find(|entry| !seen.insert(entry.id)) {
            return Err(batch_error(
                ErrorCode::BatchEntryIdsNotDistinct,
                &format!("entry `Id` `{}` is used twice", entry.id),
            ));
        }
        Ok((name, entries))
    }

    /// Handles the entries in order; a failing entry does not stop the rest.
    fn run_batch(
        &mut self,
        entries: &[Entry<'_>],
        mut handle: impl FnMut(&mut Self, &Entry<'_>) -> Result<Members>,
    ) -> Result<Members> {
        let mut successful = Vec::new();
        let mut failed = Vec::new();
        for entry in entries {
            match handle(self, entry) {
                Ok(mut result) => {
                    result.insert("Id".into(), json!(entry.id));
                    successful.push(Value::Object(result));
                }
                Err(error) => failed.push(failed_json(entry.id, &error)),
            }
        }
        Ok(members([
            ("Successful", Value::Array(successful)),
            ("Failed", Value::Array(failed)),
        ]))
    }
}
