//! Batch commands (SPEC-feature section 2).

use serde_json::{Value, json};

use super::message_commands::SendArgs;
use super::{Broker, Members, members};
use crate::error::{Error, ErrorCode, Result};
use crate::naming;
use crate::request::Params;

const MAX_ENTRIES: usize = 10;
const MAX_ENTRY_ID_CHARS: usize = 80;

/// One batch entry whose `Id` has been checked.
struct Entry<'a> {
    id: &'a str,
    params: Params<'a>,
}

fn invalid_id(message: &str) -> Error {
    Error::new(ErrorCode::InvalidBatchEntryId, message)
}

/// Applies the numbered rules 1 to 4 of the batch commands.
fn parse_entries<'a>(entries: &'a [Value]) -> Result<Vec<Entry<'a>>> {
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
                .ok_or_else(|| invalid_id("an entry is not an object"))?;
            let params = Params::new(object);
            let id = params
                .string("Id")
                .ok()
                .flatten()
                .filter(|id| naming::is_identifier(id, MAX_ENTRY_ID_CHARS))
                .ok_or_else(|| invalid_id("an entry has no valid `Id`"))?;
            Ok(Entry { id, params })
        })
        .collect::<Result<Vec<_>>>()?;
    for (i, entry) in parsed.iter().enumerate() {
        if parsed[..i].iter().any(|earlier| earlier.id == entry.id) {
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
            entry.params.require(&["Body"])?;
            let send = SendArgs::parse(entry.params)?;
            broker.send_to(name, &send)
        })
    }

    pub(super) fn delete_message_batch(&mut self, params: Params<'_>) -> Result<Members> {
        self.run_batch(params, |broker, name, entry| {
            entry.params.require(&["ReceiptHandle"])?;
            let handle = entry.params.string("ReceiptHandle")?.expect("required");
            broker.delete_handle(name, handle)
        })
    }

    /// Checks the request as a whole, then runs `handle` on each entry in
    /// order and collects the outcomes.
    fn run_batch(
        &mut self,
        params: Params<'_>,
        mut handle: impl FnMut(&mut Self, &str, &Entry<'_>) -> Result<Members>,
    ) -> Result<Members> {
        params.require(&["QueueName", "Entries"])?;
        let name = params.string("QueueName")?.expect("required");
        let entries = params.array("Entries")?.expect("required");
        self.queue_mut(name)?;
        let entries = parse_entries(entries)?;

        let mut successful = Vec::new();
        let mut failed = Vec::new();
        for entry in &entries {
            match handle(self, name, entry) {
                Ok(mut result) => {
                    result.insert("Id".into(), json!(entry.id));
                    successful.push(Value::Object(result));
                }
                Err(error) => failed.push(json!({ "Id": entry.id, "error": error.to_json() })),
            }
        }
        Ok(members([
            ("Successful", Value::Array(successful)),
            ("Failed", Value::Array(failed)),
        ]))
    }
}
