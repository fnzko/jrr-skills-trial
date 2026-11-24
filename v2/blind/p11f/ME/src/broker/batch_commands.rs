//! SendMessageBatch and DeleteMessageBatch (feature spec section 2).

use std::collections::HashSet;

use serde_json::Value;

use super::{Broker, Members, members};
use crate::error::{Error, ErrorCode, Result};
use crate::naming;
use crate::request::Params;

const MAX_ENTRIES: usize = 10;
const MAX_ENTRY_ID_CHARS: usize = 80;

/// Handles one entry of a batch as the single command would, from step 2 of
/// the check order on.
type EntryHandler = fn(&mut Broker, &str, Params<'_>) -> Result<Members>;

fn invalid_entry_id(message: &str) -> Error {
    Error::new(ErrorCode::InvalidBatchEntryId, message)
}

/// The `Id` of an entry, which must be an object.
fn entry_id<'a>(element: &'a Value) -> Result<(&'a str, Params<'a>)> {
    let entry = element
        .as_object()
        .map(Params::new)
        .ok_or_else(|| invalid_entry_id("a batch entry must be an object"))?;
    match entry.string("Id") {
        Ok(Some(id)) if naming::is_identifier(id, MAX_ENTRY_ID_CHARS) => Ok((id, entry)),
        _ => Err(invalid_entry_id(
            "`Id` must be a string of 1 to 80 characters from A-Z a-z 0-9 _ -",
        )),
    }
}

impl Broker {
    pub(super) fn send_message_batch(&mut self, params: Params<'_>) -> Result<Members> {
        self.batch(params, |broker, name, entry| {
            entry.require(&["Body"])?;
            broker.send_entry(name, entry)
        })
    }

    pub(super) fn delete_message_batch(&mut self, params: Params<'_>) -> Result<Members> {
        self.batch(params, |broker, name, entry| {
            entry.require(&["ReceiptHandle"])?;
            broker.delete_entry(name, entry)
        })
    }

    fn batch(&mut self, params: Params<'_>, handle: EntryHandler) -> Result<Members> {
        params.require(&["QueueName", "Entries"])?;
        let name = params.string("QueueName")?.expect("required");
        let elements = params.array("Entries")?.expect("required");
        self.queue_mut(name)?;

        if elements.is_empty() {
            return Err(Error::new(
                ErrorCode::EmptyBatchRequest,
                "`Entries` has no entry",
            ));
        }
        if elements.len() > MAX_ENTRIES {
            return Err(Error::new(
                ErrorCode::TooManyEntriesInBatchRequest,
                format!("`Entries` has more than {MAX_ENTRIES} entries"),
            ));
        }
        let entries = elements.iter().map(entry_id).collect::<Result<Vec<_>>>()?;
        let mut seen = HashSet::new();
        if let Some((id, _)) = entries.iter().find(|(id, _)| !seen.insert(*id)) {
            return Err(Error::new(
                ErrorCode::BatchEntryIdsNotDistinct,
                format!("entry `Id` `{id}` is used twice"),
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
                Err(error) => failed.push(serde_json::json!({"Id": id, "error": error.to_json()})),
            }
        }
        Ok(members([
            ("Successful", Value::Array(successful)),
            ("Failed", Value::Array(failed)),
        ]))
    }
}
