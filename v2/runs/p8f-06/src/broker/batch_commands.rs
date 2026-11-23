//! SendMessageBatch and DeleteMessageBatch (feature spec section 2).

use std::collections::HashSet;

use serde_json::{Map, Value, json};

use super::{Broker, Members, members};
use crate::error::{Error, ErrorCode, Result};
use crate::request::Params;

const MAX_ENTRIES: usize = 10;
const MAX_ID_CHARS: usize = 80;

/// A batch entry that passed the `Id` checks.
struct Entry<'a> {
    id: &'a str,
    params: Params<'a>,
}

fn batch_error(code: ErrorCode, message: &str) -> Error {
    Error::new(code, message)
}

/// Applies the numbered batch rules 1 to 4 to the `Entries` array.
fn parse_entries(entries: &[Value]) -> Result<Vec<Entry<'_>>> {
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
    let parsed = entries
        .iter()
        .map(|entry| {
            let invalid = || {
                batch_error(
                    ErrorCode::InvalidBatchEntryId,
                    "every entry must be an object with a valid `Id`",
                )
            };
            let members: &Map<String, Value> = entry.as_object().ok_or_else(invalid)?;
            let params = Params::new(members);
            let id = params
                .identifier("Id", MAX_ID_CHARS)
                .ok()
                .flatten()
                .ok_or_else(invalid)?;
            Ok(Entry { id, params })
        })
        .collect::<Result<Vec<_>>>()?;
    let mut seen = HashSet::new();
    if parsed.iter().any(|entry| !seen.insert(entry.id)) {
        return Err(batch_error(
            ErrorCode::BatchEntryIdsNotDistinct,
            "two entries have the same `Id`",
        ));
    }
    Ok(parsed)
}

impl Broker {
    pub(super) fn send_message_batch(&mut self, params: Params<'_>) -> Result<Members> {
        self.batch(params, Self::send_one)
    }

    pub(super) fn delete_message_batch(&mut self, params: Params<'_>) -> Result<Members> {
        self.batch(params, Self::delete_one)
    }

    /// Runs `handle` for each entry in order; one entry failing does not
    /// stop the others.
    fn batch(
        &mut self,
        params: Params<'_>,
        handle: fn(&mut Self, &str, Params<'_>) -> Result<Members>,
    ) -> Result<Members> {
        params.require(&["QueueName", "Entries"])?;
        let name = params.string("QueueName")?.expect("required");
        let entries = params.array("Entries")?.expect("required");
        self.queue_mut(name)?;
        let entries = parse_entries(entries)?;

        let mut successful = Vec::new();
        let mut failed = Vec::new();
        for Entry { id, params } in entries {
            match handle(self, name, params) {
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
