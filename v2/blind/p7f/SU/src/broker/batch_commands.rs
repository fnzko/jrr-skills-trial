//! SendMessageBatch and DeleteMessageBatch (SPEC-feature section 2).
//!
//! Each entry is run through the single-message command on a synthesized
//! parameter object, so it behaves exactly like the single command.

use std::collections::HashSet;

use serde_json::{Map, Value, json};

use super::{Broker, Members, members};
use crate::error::{Error, ErrorCode, Result};
use crate::naming;
use crate::request::Params;

const MAX_ENTRIES: usize = 10;
const MAX_ID_CHARS: usize = 80;

fn batch_error(code: ErrorCode, message: &str) -> Error {
    Error::new(code, message)
}

impl Broker {
    pub(super) fn batch(
        &mut self,
        params: Params<'_>,
        single: fn(&mut Self, Params<'_>) -> Result<Members>,
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
        let mut ids = Vec::new();
        for entry in entries {
            let id = entry
                .as_object()
                .and_then(|o| o.get("Id"))
                .and_then(Value::as_str)
                .filter(|id| naming::is_identifier(id, MAX_ID_CHARS))
                .ok_or_else(|| {
                    batch_error(
                        ErrorCode::InvalidBatchEntryId,
                        "an entry is not an object or has no valid `Id`",
                    )
                })?;
            ids.push(id);
        }
        let mut seen = HashSet::new();
        if !ids.iter().all(|id| seen.insert(*id)) {
            return Err(batch_error(
                ErrorCode::BatchEntryIdsNotDistinct,
                "two entries have the same `Id`",
            ));
        }

        let mut successful = Vec::new();
        let mut failed = Vec::new();
        for (entry, id) in entries.iter().zip(ids) {
            let mut single_members: Map<String, Value> = entry.as_object().cloned().expect("checked");
            single_members.insert("QueueName".into(), json!(name));
            match single(self, Params::new(&single_members)) {
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
