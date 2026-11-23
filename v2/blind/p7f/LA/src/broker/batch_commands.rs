//! Batch commands (SPEC-feature section 2).

use std::collections::HashSet;

use serde_json::{Map, Value, json};

use super::{Broker, Members, members};
use crate::error::{Error, ErrorCode, Result};
use crate::naming;
use crate::request::Params;

const MAX_ENTRIES: usize = 10;
const MAX_ENTRY_ID_CHARS: usize = 80;

fn invalid_id() -> Error {
    Error::new(
        ErrorCode::InvalidBatchEntryId,
        "each entry must be an object with an `Id` of 1 to 80 characters from A-Z a-z 0-9 _ -",
    )
}

impl Broker {
    /// Runs `single` once per entry, as if the entry were a command with the
    /// same `QueueName`, and collects the outcomes in entry order.
    pub(super) fn batch(
        &mut self,
        params: Params<'_>,
        single: fn(&mut Broker, Params<'_>) -> Result<Members>,
    ) -> Result<Members> {
        params.require(&["QueueName", "Entries"])?;
        let name = params.string("QueueName")?.expect("required");
        let entries = params.array("Entries")?.expect("required");
        self.queue_mut(name)?;

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
        let entries = entries
            .iter()
            .map(|entry| {
                let object = entry.as_object().ok_or_else(invalid_id)?;
                let id = object
                    .get("Id")
                    .and_then(Value::as_str)
                    .filter(|id| naming::is_identifier(id, MAX_ENTRY_ID_CHARS))
                    .ok_or_else(invalid_id)?;
                Ok((id, object))
            })
            .collect::<Result<Vec<_>>>()?;
        let mut seen = HashSet::new();
        if !entries.iter().all(|(id, _)| seen.insert(*id)) {
            return Err(Error::new(
                ErrorCode::BatchEntryIdsNotDistinct,
                "two entries have the same `Id`",
            ));
        }

        let mut successful = Vec::new();
        let mut failed = Vec::new();
        for (id, object) in entries {
            let mut command: Map<String, Value> = object.clone();
            command.insert("QueueName".into(), json!(name));
            match single(self, Params::new(&command)) {
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
