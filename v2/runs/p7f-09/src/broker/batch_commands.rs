//! Batch commands (SPEC-feature section 2).

use std::collections::HashSet;

use serde_json::{Value, json};

use super::message_commands::SendArgs;
use super::{Broker, Members, members};
use crate::error::{Error, ErrorCode, Result};
use crate::naming;
use crate::request::Params;

const MAX_ENTRIES: usize = 10;
const MAX_ENTRY_ID_CHARS: usize = 80;

fn batch_error(code: ErrorCode, message: &str) -> Error {
    Error::new(code, message)
}

fn error_json(error: &Error) -> Value {
    json!({ "code": error.code.as_str(), "message": error.message })
}

/// The entries of a validated batch request: each `Id` with the entry's
/// own parameters.
type Entries<'a> = Vec<(&'a str, Params<'a>)>;

impl Broker {
    /// Checks everything a batch request has in common: required
    /// parameters, value types, queue lookup and the numbered rules 1 to 4.
    fn batch_request<'a>(&mut self, params: Params<'a>) -> Result<(&'a str, Entries<'a>)> {
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
        let mut parsed = Entries::new();
        for entry in entries {
            let invalid_id = || {
                batch_error(
                    ErrorCode::InvalidBatchEntryId,
                    "every entry must be an object with a valid `Id`",
                )
            };
            let object = entry.as_object().ok_or_else(invalid_id)?;
            let params = Params::new(object);
            let id = match object.get("Id") {
                Some(Value::String(id)) if naming::is_identifier(id, MAX_ENTRY_ID_CHARS) => id,
                _ => return Err(invalid_id()),
            };
            parsed.push((id.as_str(), params));
        }
        let mut seen = HashSet::new();
        if !parsed.iter().all(|(id, _)| seen.insert(*id)) {
            return Err(batch_error(
                ErrorCode::BatchEntryIdsNotDistinct,
                "two entries have the same `Id`",
            ));
        }
        Ok((name, parsed))
    }

    pub(super) fn send_message_batch(&mut self, params: Params<'_>) -> Result<Members> {
        let (name, entries) = self.batch_request(params)?;
        Ok(self.run_batch(entries, |broker, entry| {
            let args = SendArgs::parse(entry)?;
            broker.send_to(name, args)
        }))
    }

    pub(super) fn delete_message_batch(&mut self, params: Params<'_>) -> Result<Members> {
        let (name, entries) = self.batch_request(params)?;
        Ok(self.run_batch(entries, |broker, entry| {
            entry.require(&["ReceiptHandle"])?;
            let handle = entry.string("ReceiptHandle")?.expect("required");
            broker.delete_handle(name, handle)
        }))
    }

    /// Handles the entries in order and sorts them into `Successful` and
    /// `Failed`.
    fn run_batch<'a>(
        &mut self,
        entries: Entries<'a>,
        mut handle: impl FnMut(&mut Self, Params<'a>) -> Result<Members>,
    ) -> Members {
        let mut successful = Vec::new();
        let mut failed = Vec::new();
        for (id, entry) in entries {
            match handle(self, entry) {
                Ok(mut result) => {
                    result.insert("Id".into(), json!(id));
                    successful.push(Value::Object(result));
                }
                Err(error) => failed.push(json!({ "Id": id, "error": error_json(&error) })),
            }
        }
        members([
            ("Successful", Value::Array(successful)),
            ("Failed", Value::Array(failed)),
        ])
    }
}
