//! Batch commands (SPEC-feature section 2).

use std::collections::HashSet;

use serde_json::Value;

use super::message_commands::SendParams;
use super::{Broker, Members};
use crate::error::{Error, ErrorCode, Result};
use crate::request::Params;

const MAX_ENTRIES: usize = 10;
const MAX_ID_CHARS: usize = 80;

fn batch_error(code: ErrorCode, message: &str) -> Error {
    Error::new(code, message)
}

/// An entry's `Id` and parameters, or `None` if the entry is not an object
/// or has no valid `Id`.
fn parse_entry(entry: &Value) -> Option<(&str, Params<'_>)> {
    let params = Params::new(entry.as_object()?);
    match params.identifier("Id", MAX_ID_CHARS) {
        Ok(id) => Some((id?, params)),
        Err(_) => None,
    }
}

impl Broker {
    pub(super) fn send_message_batch(&mut self, params: Params<'_>) -> Result<Members> {
        self.batch(params, |broker, name, entry| {
            let send = SendParams::parse(entry)?;
            broker.send_to_queue(name, send)
        })
    }

    pub(super) fn delete_message_batch(&mut self, params: Params<'_>) -> Result<Members> {
        self.batch(params, |broker, name, entry| {
            entry.require(&["ReceiptHandle"])?;
            let handle = entry.string("ReceiptHandle")?.expect("required");
            broker.delete_by_handle(name, handle)
        })
    }

    /// Validates a batch request and runs `handle` for each entry in order.
    /// `handle` does what the single command does from check 2 on.
    fn batch(
        &mut self,
        params: Params<'_>,
        handle: impl Fn(&mut Self, &str, Params<'_>) -> Result<Members>,
    ) -> Result<Members> {
        params.require(&["QueueName", "Entries"])?;
        let name = params.string("QueueName")?.expect("required");
        let entries = params.array("Entries")?.expect("required");
        self.queue_mut(name)?;

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
        let entries = entries
            .iter()
            .map(|entry| {
                parse_entry(entry).ok_or_else(|| {
                    batch_error(
                        ErrorCode::InvalidBatchEntryId,
                        "an entry is not an object or has no valid `Id`",
                    )
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let mut seen = HashSet::new();
        if !entries.iter().all(|(id, _)| seen.insert(*id)) {
            return Err(batch_error(
                ErrorCode::BatchEntryIdsNotDistinct,
                "two entries have the same `Id`",
            ));
        }

        let mut successful = Vec::new();
        let mut failed = Vec::new();
        for (id, entry) in entries {
            match handle(self, name, entry) {
                Ok(mut result) => {
                    result.insert("Id".into(), Value::from(id));
                    successful.push(Value::Object(result));
                }
                Err(error) => {
                    failed.push(serde_json::json!({"Id": id, "error": error.to_json()}));
                }
            }
        }
        Ok(super::members([
            ("Successful", Value::Array(successful)),
            ("Failed", Value::Array(failed)),
        ]))
    }
}
