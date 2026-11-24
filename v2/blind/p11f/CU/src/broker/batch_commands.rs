//! `SendMessageBatch` and `DeleteMessageBatch` (SPEC-feature section 2).

use serde_json::{Value, json};

use super::message_commands::{IdCounter, SendArgs, delete_in_queue, send_to_queue};
use super::{Broker, Members, lookup, members};
use crate::error::{Error, ErrorCode, Result};
use crate::naming;
use crate::request::Params;

const MAX_ENTRIES: usize = 10;
const MAX_ENTRY_ID_CHARS: usize = 80;

fn batch_error(code: ErrorCode, message: &str) -> Error {
    Error::new(code, message)
}

/// Checks the numbered batch rules 1 to 4 and returns each entry's `Id` with
/// its parameters.
fn validate_entries(entries: &[Value]) -> Result<Vec<(&str, Params<'_>)>> {
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
    let mut parsed = Vec::with_capacity(entries.len());
    for entry in entries {
        let invalid = || {
            batch_error(
                ErrorCode::InvalidBatchEntryId,
                "every entry must be an object with a valid `Id`",
            )
        };
        let object = entry.as_object().ok_or_else(invalid)?;
        let params = Params::new(object);
        let id = params
            .string("Id")
            .ok()
            .flatten()
            .filter(|id| naming::is_identifier(id, MAX_ENTRY_ID_CHARS))
            .ok_or_else(invalid)?;
        parsed.push((id, params));
    }
    for (i, (id, _)) in parsed.iter().enumerate() {
        if parsed[..i].iter().any(|(earlier, _)| earlier == id) {
            return Err(batch_error(
                ErrorCode::BatchEntryIdsNotDistinct,
                "two entries have the same `Id`",
            ));
        }
    }
    Ok(parsed)
}

/// Builds the `Successful` / `Failed` result from per-entry outcomes.
fn batch_result(outcomes: Vec<(&str, Result<Members>)>) -> Members {
    let (mut successful, mut failed) = (Vec::new(), Vec::new());
    for (id, outcome) in outcomes {
        match outcome {
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
    members([
        ("Successful", Value::Array(successful)),
        ("Failed", Value::Array(failed)),
    ])
}

/// Steps 2 and 3 of the check order for the two batch parameters.
fn batch_params<'a>(params: Params<'a>) -> Result<(&'a str, &'a [Value])> {
    params.require(&["QueueName", "Entries"])?;
    let name = params.string("QueueName")?.expect("required");
    let entries = params.array("Entries")?.expect("required");
    Ok((name, entries))
}

impl Broker {
    pub(super) fn send_message_batch(&mut self, params: Params<'_>) -> Result<Members> {
        let (name, entries) = batch_params(params)?;
        let now = self.now;
        let queue = lookup(&mut self.queues, now, name)?;
        let entries = validate_entries(entries)?;

        let outcomes = entries
            .into_iter()
            .map(|(id, entry)| {
                let outcome = SendArgs::parse(entry).and_then(|args| {
                    send_to_queue(queue, IdCounter(&mut self.next_message_id), now, args)
                });
                (id, outcome)
            })
            .collect();
        Ok(batch_result(outcomes))
    }

    pub(super) fn delete_message_batch(&mut self, params: Params<'_>) -> Result<Members> {
        let (name, entries) = batch_params(params)?;
        let (now, issued) = (self.now, self.next_receipt_handle);
        let queue = lookup(&mut self.queues, now, name)?;
        let entries = validate_entries(entries)?;

        let outcomes = entries
            .into_iter()
            .map(|(id, entry)| {
                let outcome = entry
                    .require(&["ReceiptHandle"])
                    .and_then(|()| entry.string("ReceiptHandle"))
                    .and_then(|handle| {
                        delete_in_queue(queue, issued, now, handle.expect("required"))
                    });
                (id, outcome)
            })
            .collect();
        Ok(batch_result(outcomes))
    }
}
