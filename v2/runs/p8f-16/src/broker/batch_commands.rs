//! Batch message commands (SPEC-feature section 2).

use std::collections::HashSet;

use serde_json::{Map, Value, json};

use super::{Broker, Members};
use crate::error::{Error, ErrorCode, Result};
use crate::naming;
use crate::request::Params;

const MAX_ENTRIES: usize = 10;
const MAX_ENTRY_ID_CHARS: usize = 80;

/// One validated batch entry: its `Id` and its parameters.
struct Entry<'a> {
    id: &'a str,
    members: &'a Map<String, Value>,
}

fn batch_error(code: ErrorCode, message: &str) -> Error {
    Error::new(code, message)
}

fn entry_of(value: &Value) -> Result<Entry<'_>> {
    let invalid = || {
        batch_error(
            ErrorCode::InvalidBatchEntryId,
            "every entry must be an object with an `Id` of 1 to 80 characters from A-Z a-z 0-9 _ -",
        )
    };
    let members = value.as_object().ok_or_else(invalid)?;
    let id = members
        .get("Id")
        .and_then(Value::as_str)
        .filter(|id| naming::is_identifier(id, MAX_ENTRY_ID_CHARS))
        .ok_or_else(invalid)?;
    Ok(Entry { id, members })
}

/// Applies the numbered rules of a batch request to its `Entries`.
fn entries_of(values: &[Value]) -> Result<Vec<Entry<'_>>> {
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
    let entries = values.iter().map(entry_of).collect::<Result<Vec<_>>>()?;
    let mut seen = HashSet::with_capacity(entries.len());
    if !entries.iter().all(|entry| seen.insert(entry.id)) {
        return Err(batch_error(
            ErrorCode::BatchEntryIdsNotDistinct,
            "two entries have the same `Id`",
        ));
    }
    Ok(entries)
}

impl Broker {
    pub(super) fn send_message_batch(&mut self, params: Params<'_>) -> Result<Members> {
        self.run_batch(params, |broker, name, entry| {
            broker.send_to(name, Params::new(entry))
        })
    }

    pub(super) fn delete_message_batch(&mut self, params: Params<'_>) -> Result<Members> {
        self.run_batch(params, |broker, name, entry| {
            broker.delete_from(name, Params::new(entry))
        })
    }

    /// Checks a batch request, then runs `handle` on each entry in order and
    /// sorts the outcomes into `Successful` and `Failed`.
    fn run_batch(
        &mut self,
        params: Params<'_>,
        mut handle: impl FnMut(&mut Self, &str, &Map<String, Value>) -> Result<Members>,
    ) -> Result<Members> {
        params.require(&["QueueName", "Entries"])?;
        let name = params.string("QueueName")?.expect("required");
        let values = params.array("Entries")?.expect("required");
        self.queue_mut(name)?;
        let entries = entries_of(values)?;

        let mut successful = Vec::new();
        let mut failed = Vec::new();
        for entry in entries {
            match handle(self, name, entry.members) {
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
        let mut result = Members::new();
        result.insert("Successful".into(), Value::Array(successful));
        result.insert("Failed".into(), Value::Array(failed));
        Ok(result)
    }
}
