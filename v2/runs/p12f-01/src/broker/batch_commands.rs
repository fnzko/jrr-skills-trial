//! SendMessageBatch and DeleteMessageBatch (SPEC-feature section 2).

use std::collections::HashSet;

use serde_json::{Value, json};

use super::{Broker, Members, members};
use crate::error::{Error, ErrorCode, Result};
use crate::request::Params;

const MAX_ENTRIES: usize = 10;
const MAX_ENTRY_ID_CHARS: usize = 80;

fn invalid_entry_id(message: &str) -> Error {
    Error::new(ErrorCode::InvalidBatchEntryId, message)
}

/// An entry's `Id` and its parameters.
struct Entry<'a> {
    id: &'a str,
    params: Params<'a>,
}

/// Applies the numbered rules 1 to 4 of a batch request.
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
                .identifier("Id", MAX_ENTRY_ID_CHARS)
                .map_err(|e| invalid_entry_id(&e.message))?
                .ok_or_else(|| invalid_entry_id("an entry has no `Id`"))?;
            Ok(Entry { id, params })
        })
        .collect::<Result<Vec<_>>>()?;
    let mut seen = HashSet::with_capacity(parsed.len());
    for entry in &parsed {
        if !seen.insert(entry.id) {
            return Err(Error::new(
                ErrorCode::BatchEntryIdsNotDistinct,
                format!("`Id` `{}` is used twice", entry.id),
            ));
        }
    }
    Ok(parsed)
}

impl Broker {
    pub(super) fn send_message_batch(&mut self, params: Params<'_>) -> Result<Members> {
        self.run_batch(params, |broker, name, entry| {
            entry.require(&["Body"])?;
            broker.send_entry(name, entry)
        })
    }

    pub(super) fn delete_message_batch(&mut self, params: Params<'_>) -> Result<Members> {
        self.run_batch(params, |broker, name, entry| {
            entry.require(&["ReceiptHandle"])?;
            let handle = entry.required_string("ReceiptHandle")?;
            broker.delete_entry(name, handle)
        })
    }

    /// Checks the batch request, then runs `handle` for each entry in order
    /// and sorts the outcomes into `Successful` and `Failed`.
    fn run_batch(
        &mut self,
        params: Params<'_>,
        handle: impl Fn(&mut Self, &str, Params<'_>) -> Result<Members>,
    ) -> Result<Members> {
        params.require(&["QueueName", "Entries"])?;
        let name = params.required_string("QueueName")?;
        let entries = params.required_array("Entries")?;
        self.queue_mut(name)?;
        let entries = parse_entries(entries)?;

        let mut successful = Vec::new();
        let mut failed = Vec::new();
        for entry in entries {
            match handle(self, name, entry.params) {
                Ok(mut result) => {
                    result.insert("Id".into(), json!(entry.id));
                    successful.push(Value::Object(result));
                }
                Err(Error { code, message }) => failed.push(json!({
                    "Id": entry.id,
                    "error": { "code": code.as_str(), "message": message },
                })),
            }
        }
        Ok(members([
            ("Successful", Value::Array(successful)),
            ("Failed", Value::Array(failed)),
        ]))
    }
}
