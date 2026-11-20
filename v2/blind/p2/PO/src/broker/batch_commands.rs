//! SendMessageBatch and DeleteMessageBatch (SPEC-feature section 2).

use std::collections::HashSet;

use serde_json::{Value, json};

use super::{Broker, Members, members};
use crate::error::{Error, ErrorCode, Result};
use crate::request::Params;

const MAX_ENTRIES: usize = 10;
const MAX_ID_CHARS: usize = 80;

/// Validates the `Entries` rules 1 to 4 and returns each entry's id and
/// parameters in order.
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
            "`Entries` has more than 10 entries",
        ));
    }
    let mut parsed = Vec::with_capacity(entries.len());
    for entry in entries {
        let invalid = || {
            Error::new(
                ErrorCode::InvalidBatchEntryId,
                "every entry must be an object with an `Id` of 1 to 80 characters from A-Z a-z 0-9 _ -",
            )
        };
        let params = Params::new(entry.as_object().ok_or_else(invalid)?);
        // Any problem with `Id` (absent, not a string, bad characters) is the same error.
        let id = params
            .identifier("Id", MAX_ID_CHARS)
            .ok()
            .flatten()
            .ok_or_else(invalid)?;
        parsed.push((id, params));
    }
    let mut seen = HashSet::new();
    if !parsed.iter().all(|(id, _)| seen.insert(*id)) {
        return Err(Error::new(
            ErrorCode::BatchEntryIdsNotDistinct,
            "two entries have the same `Id`",
        ));
    }
    Ok(parsed)
}

impl Broker {
    pub(super) fn send_message_batch(&mut self, params: Params<'_>) -> Result<Members> {
        self.run_batch(params, Self::send_to)
    }

    pub(super) fn delete_message_batch(&mut self, params: Params<'_>) -> Result<Members> {
        self.run_batch(params, Self::delete_from)
    }

    /// Checks the request, then runs `handle` for each entry in order. An
    /// entry's failure is reported in `Failed` and the others carry on.
    fn run_batch(
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
        for (id, entry) in entries {
            match handle(self, name, entry) {
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
