//! SendMessageBatch and DeleteMessageBatch (feature spec section 2).

use std::collections::HashSet;

use serde_json::{Value, json};

use super::{Broker, Members};
use crate::error::{Error, ErrorCode, Result};
use crate::naming;
use crate::request::Params;

const MAX_ENTRIES: usize = 10;
const MAX_ENTRY_ID_CHARS: usize = 80;

/// A batch entry `Id` that follows the `Id` rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct EntryId<'a>(&'a str);

/// One batch entry whose `Id` has passed the batch rules.
struct Entry<'a> {
    id: EntryId<'a>,
    params: Params<'a>,
}

fn invalid_entry_id(message: &str) -> Error {
    Error::new(ErrorCode::InvalidBatchEntryId, message)
}

/// Applies batch rules 1 to 4 to the raw `Entries` array.
fn parse_entries(raw: &[Value]) -> Result<Vec<Entry<'_>>> {
    if raw.is_empty() {
        return Err(Error::new(
            ErrorCode::EmptyBatchRequest,
            "`Entries` has no entry",
        ));
    }
    if raw.len() > MAX_ENTRIES {
        return Err(Error::new(
            ErrorCode::TooManyEntriesInBatchRequest,
            format!("`Entries` has more than {MAX_ENTRIES} entries"),
        ));
    }
    let entries = raw
        .iter()
        .map(|value| {
            let members = value
                .as_object()
                .ok_or_else(|| invalid_entry_id("an entry is not an object"))?;
            let params = Params::new(members);
            let bad_id = || {
                invalid_entry_id("an entry `Id` must be 1 to 80 characters from A-Z a-z 0-9 _ -")
            };
            let id = params
                .string("Id")
                .map_err(|_| bad_id())?
                .filter(|id| naming::is_identifier(id, MAX_ENTRY_ID_CHARS))
                .map(EntryId)
                .ok_or_else(bad_id)?;
            Ok(Entry { id, params })
        })
        .collect::<Result<Vec<_>>>()?;
    let mut seen = HashSet::new();
    if let Some(entry) = entries.iter().find(|entry| !seen.insert(entry.id)) {
        return Err(Error::new(
            ErrorCode::BatchEntryIdsNotDistinct,
            format!("entry `Id` `{}` is used twice", entry.id.0),
        ));
    }
    Ok(entries)
}

impl Broker {
    pub(super) fn send_message_batch(&mut self, params: Params<'_>) -> Result<Members> {
        self.batch(params, |broker, name, params| {
            broker.send_entry(name, params)
        })
    }

    pub(super) fn delete_message_batch(&mut self, params: Params<'_>) -> Result<Members> {
        self.batch(params, |broker, name, params| {
            broker.delete_entry(name, params)
        })
    }

    /// Runs `handle` on each entry in order and collects the outcomes.
    fn batch(
        &mut self,
        params: Params<'_>,
        mut handle: impl FnMut(&mut Self, &str, Params<'_>) -> Result<Members>,
    ) -> Result<Members> {
        params.require(&["QueueName", "Entries"])?;
        let name = params.required_string("QueueName")?;
        let raw = params.required_array("Entries")?;
        self.queue_mut(name)?;
        let entries = parse_entries(raw)?;

        let mut successful = Vec::new();
        let mut failed = Vec::new();
        for entry in entries {
            match handle(self, name, entry.params) {
                Ok(mut result) => {
                    result.insert("Id".into(), json!(entry.id.0));
                    successful.push(Value::Object(result));
                }
                Err(Error { code, message }) => failed.push(json!({
                    "Id": entry.id.0,
                    "error": { "code": code.as_str(), "message": message },
                })),
            }
        }
        Ok(super::members([
            ("Successful", Value::Array(successful)),
            ("Failed", Value::Array(failed)),
        ]))
    }
}
