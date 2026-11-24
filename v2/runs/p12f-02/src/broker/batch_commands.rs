//! SendMessageBatch and DeleteMessageBatch (SPEC-feature section 2).

use std::collections::HashSet;

use serde_json::{Value, json};

use super::{Broker, Members};
use crate::error::{Error, ErrorCode, Result};
use crate::naming;
use crate::request::Params;

const MAX_ENTRIES: usize = 10;
const MAX_ENTRY_ID_CHARS: usize = 80;

/// The `Id` of a batch entry: 1 to 80 characters from `A-Z a-z 0-9 _ -`.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct EntryId<'a>(&'a str);

impl<'a> EntryId<'a> {
    fn of(entry: Params<'a>) -> Result<Self> {
        entry
            .get("Id")
            .and_then(Value::as_str)
            .filter(|id| naming::is_identifier(id, MAX_ENTRY_ID_CHARS))
            .map(Self)
            .ok_or_else(|| {
                Error::new(
                    ErrorCode::InvalidBatchEntryId,
                    format!("an entry needs an `Id` of 1 to {MAX_ENTRY_ID_CHARS} characters from A-Z a-z 0-9 _ -"),
                )
            })
    }
}

/// Checks rules 1 to 4 and returns the entries in order.
fn parse_entries(entries: &[Value]) -> Result<Vec<(EntryId<'_>, Params<'_>)>> {
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
            let members = entry.as_object().ok_or_else(|| {
                Error::new(ErrorCode::InvalidBatchEntryId, "an entry is not an object")
            })?;
            let params = Params::new(members);
            Ok((EntryId::of(params)?, params))
        })
        .collect::<Result<Vec<_>>>()?;
    let mut seen = HashSet::new();
    if let Some((EntryId(id), _)) = parsed.iter().find(|(id, _)| !seen.insert(*id)) {
        return Err(Error::new(
            ErrorCode::BatchEntryIdsNotDistinct,
            format!("the entry `Id` `{id}` is used twice"),
        ));
    }
    Ok(parsed)
}

impl Broker {
    pub(super) fn send_message_batch(&mut self, params: Params<'_>) -> Result<Members> {
        self.run_batch(params, |broker, name, entry| broker.send_to(name, entry))
    }

    pub(super) fn delete_message_batch(&mut self, params: Params<'_>) -> Result<Members> {
        self.run_batch(params, |broker, name, entry| {
            broker.delete_from(name, entry)
        })
    }

    /// Runs `handle` for each entry after the batch-level checks. An entry
    /// that fails is reported and does not stop the others.
    fn run_batch(
        &mut self,
        params: Params<'_>,
        mut handle: impl FnMut(&mut Self, &str, Params<'_>) -> Result<Members>,
    ) -> Result<Members> {
        params.require(&["QueueName", "Entries"])?;
        let name = params.required_string("QueueName")?;
        let entries = params.required_array("Entries")?;
        self.queue_mut(name)?;
        let entries = parse_entries(entries)?;

        let mut successful = Vec::new();
        let mut failed = Vec::new();
        for (EntryId(id), entry) in entries {
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
        Ok(super::members([
            ("Successful", Value::Array(successful)),
            ("Failed", Value::Array(failed)),
        ]))
    }
}
