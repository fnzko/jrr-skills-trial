//! SendMessageBatch and DeleteMessageBatch (SPEC-feature section 2).

use std::collections::HashSet;

use serde_json::{Map, Value, json};

use super::message_commands::SendArgs;
use super::{Broker, Members, members};
use crate::error::{Error, ErrorCode, Result};
use crate::request::Params;

const MAX_ENTRIES: usize = 10;
const MAX_ID_CHARS: usize = 80;

/// One batch entry: its validated `Id` and its members.
struct Entry<'a> {
    id: &'a str,
    members: &'a Map<String, Value>,
}

fn invalid_id(message: &str) -> Error {
    Error::new(ErrorCode::InvalidBatchEntryId, message)
}

/// Applies the numbered rules 1 to 4 that every batch command shares.
fn parse_entries(entries: &[Value]) -> Result<Vec<Entry<'_>>> {
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
            let members = entry
                .as_object()
                .ok_or_else(|| invalid_id("an entry is not an object"))?;
            let id = Params::new(members)
                .identifier("Id", MAX_ID_CHARS)
                .ok()
                .flatten()
                .ok_or_else(|| invalid_id("an entry has no valid `Id`"))?;
            Ok(Entry { id, members })
        })
        .collect::<Result<Vec<_>>>()?;
    let mut seen = HashSet::new();
    if let Some(entry) = parsed.iter().find(|entry| !seen.insert(entry.id)) {
        return Err(Error::new(
            ErrorCode::BatchEntryIdsNotDistinct,
            format!("entry `Id` `{}` is used twice", entry.id),
        ));
    }
    Ok(parsed)
}

/// Collects per-entry outcomes into the `Successful` and `Failed` members.
fn outcome(results: Vec<(&str, Result<Members>)>) -> Members {
    let mut successful = Vec::new();
    let mut failed = Vec::new();
    for (id, result) in results {
        match result {
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

impl Broker {
    pub(super) fn send_message_batch(&mut self, params: Params<'_>) -> Result<Members> {
        let (name, entries) = self.batch_request(params)?;
        let results = entries
            .iter()
            .map(|entry| {
                let result = SendArgs::parse(Params::new(entry.members))
                    .and_then(|args| self.send_to(name, args));
                (entry.id, result)
            })
            .collect();
        Ok(outcome(results))
    }

    pub(super) fn delete_message_batch(&mut self, params: Params<'_>) -> Result<Members> {
        let (name, entries) = self.batch_request(params)?;
        let results = entries
            .iter()
            .map(|entry| {
                let params = Params::new(entry.members);
                let result = params.require(&["ReceiptHandle"]).and_then(|()| {
                    let handle = params.string("ReceiptHandle")?.expect("required");
                    self.delete_from(name, handle)
                });
                (entry.id, result)
            })
            .collect();
        Ok(outcome(results))
    }

    /// Checks 2 to 4 of SPEC section 2 and the batch rules 1 to 4.
    fn batch_request<'a>(&mut self, params: Params<'a>) -> Result<(&'a str, Vec<Entry<'a>>)> {
        params.require(&["QueueName", "Entries"])?;
        let name = params.string("QueueName")?.expect("required");
        let entries = params.array("Entries")?.expect("required");
        self.queue_mut(name)?;
        Ok((name, parse_entries(entries)?))
    }
}
