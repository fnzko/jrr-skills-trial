//! Batch commands (SPEC-feature section 2).

use std::collections::HashSet;

use serde_json::{Map, Value, json};

use super::{Broker, Members, members};
use crate::error::{Error, ErrorCode, Result};
use crate::request::Params;

const MAX_ENTRIES: usize = 10;
const MAX_ENTRY_ID_CHARS: usize = 80;

fn batch_error(code: ErrorCode, message: &str) -> Error {
    Error::new(code, message)
}

/// Checks the batch rules 1 to 4 and returns each entry's `Id` and members.
fn parse_entries<'a>(entries: &'a [Value]) -> Result<Vec<(&'a str, &'a Map<String, Value>)>> {
    if entries.is_empty() {
        return Err(batch_error(ErrorCode::EmptyBatchRequest, "`Entries` is empty"));
    }
    if entries.len() > MAX_ENTRIES {
        return Err(batch_error(
            ErrorCode::TooManyEntriesInBatchRequest,
            "`Entries` has more than 10 entries",
        ));
    }
    let mut parsed = Vec::with_capacity(entries.len());
    for entry in entries {
        let invalid = || batch_error(ErrorCode::InvalidBatchEntryId, "entry has no valid `Id`");
        let object = entry.as_object().ok_or_else(invalid)?;
        let id = Params::new(object)
            .identifier("Id", MAX_ENTRY_ID_CHARS)
            .ok()
            .flatten()
            .ok_or_else(invalid)?;
        parsed.push((id, object));
    }
    let mut seen = HashSet::new();
    if !parsed.iter().all(|(id, _)| seen.insert(*id)) {
        return Err(batch_error(
            ErrorCode::BatchEntryIdsNotDistinct,
            "entry `Id`s are not distinct",
        ));
    }
    Ok(parsed)
}

impl Broker {
    pub(super) fn send_message_batch(&mut self, params: Params<'_>) -> Result<Members> {
        self.batch(params, Self::send_message)
    }

    pub(super) fn delete_message_batch(&mut self, params: Params<'_>) -> Result<Members> {
        self.batch(params, Self::delete_message)
    }

    /// Runs each entry as the single command `single` would, with the batch's
    /// `QueueName`.
    fn batch(
        &mut self,
        params: Params<'_>,
        single: fn(&mut Self, Params<'_>) -> Result<Members>,
    ) -> Result<Members> {
        params.require(&["QueueName", "Entries"])?;
        let name = params.string("QueueName")?.expect("required");
        let entries = params.array("Entries")?.expect("required");
        self.queue_mut(name)?;
        let entries = parse_entries(entries)?;

        let mut successful = Vec::new();
        let mut failed = Vec::new();
        for (id, entry) in entries {
            let mut request = entry.clone();
            request.insert("QueueName".into(), json!(name));
            match single(self, Params::new(&request)) {
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
        Ok(members([
            ("Successful", Value::Array(successful)),
            ("Failed", Value::Array(failed)),
        ]))
    }
}
