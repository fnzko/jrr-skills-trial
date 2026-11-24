//! Batch commands (feature spec section 2).

use std::collections::HashSet;

use serde_json::{Value, json};

use super::message_commands::SendArgs;
use super::{Broker, Members, members};
use crate::error::{Error, ErrorCode, Result};
use crate::request::Params;

const MAX_ENTRIES: usize = 10;
const MAX_ENTRY_ID_CHARS: usize = 80;

/// The entries of a batch request, as `(Id, parameters)` pairs.
fn entries<'a>(elements: &'a [Value]) -> Result<Vec<(&'a str, Params<'a>)>> {
    if elements.is_empty() {
        return Err(Error::new(
            ErrorCode::EmptyBatchRequest,
            "`Entries` has no entry",
        ));
    }
    if elements.len() > MAX_ENTRIES {
        return Err(Error::new(
            ErrorCode::TooManyEntriesInBatchRequest,
            "`Entries` has more than 10 entries",
        ));
    }
    let entries = elements
        .iter()
        .map(|element| {
            let invalid = || {
                Error::new(
                    ErrorCode::InvalidBatchEntryId,
                    "an entry must be an object with an `Id` of 1 to 80 characters from A-Z a-z 0-9 _ -",
                )
            };
            let params = Params::new(element.as_object().ok_or_else(invalid)?);
            match params.identifier("Id", MAX_ENTRY_ID_CHARS) {
                Ok(Some(id)) => Ok((id, params)),
                // A wrongly typed or malformed `Id` is the same error as an absent one.
                Ok(None) | Err(_) => Err(invalid()),
            }
        })
        .collect::<Result<Vec<_>>>()?;
    let mut seen = HashSet::new();
    if !entries.iter().all(|(id, _)| seen.insert(*id)) {
        return Err(Error::new(
            ErrorCode::BatchEntryIdsNotDistinct,
            "two entries have the same `Id`",
        ));
    }
    Ok(entries)
}

impl Broker {
    pub(super) fn send_message_batch(&mut self, params: Params<'_>) -> Result<Members> {
        self.run_batch(params, |broker, queue, entry| {
            broker.send_to(queue, SendArgs::parse(entry)?)
        })
    }

    pub(super) fn delete_message_batch(&mut self, params: Params<'_>) -> Result<Members> {
        self.run_batch(params, |broker, queue, entry| {
            entry.require(&["ReceiptHandle"])?;
            let handle = entry.require_string("ReceiptHandle")?;
            broker.delete_handle(queue, handle)
        })
    }

    /// Checks the batch as a whole, then runs `handle` on each entry in
    /// order. A failing entry is reported and does not stop the others.
    fn run_batch(
        &mut self,
        params: Params<'_>,
        mut handle: impl FnMut(&mut Self, &str, Params<'_>) -> Result<Members>,
    ) -> Result<Members> {
        params.require(&["QueueName", "Entries"])?;
        let name = params.require_string("QueueName")?;
        let elements = params.require_array("Entries")?;
        self.queue_mut(name)?;
        let entries = entries(elements)?;

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
