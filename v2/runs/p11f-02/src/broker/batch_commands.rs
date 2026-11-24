//! SendMessageBatch and DeleteMessageBatch (SPEC-feature section 2).

use std::collections::HashSet;

use serde_json::{Value, json};

use super::{Broker, Members, error_json, members};
use crate::error::{Error, ErrorCode, Result};
use crate::request::Params;

const MAX_ENTRIES: usize = 10;
const MAX_ENTRY_ID_CHARS: usize = 80;

/// One batch entry: its `Id` and the entry object as parameters.
struct Entry<'a> {
    id: &'a str,
    params: Params<'a>,
}

fn invalid_entry_id() -> Error {
    Error::new(
        ErrorCode::InvalidBatchEntryId,
        format!(
            "an entry needs an `Id` of 1 to {MAX_ENTRY_ID_CHARS} characters from A-Z a-z 0-9 _ -"
        ),
    )
}

/// Applies the numbered rules 1 to 4 of the batch commands.
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
        .map(|value| {
            let params = Params::new(value.as_object().ok_or_else(invalid_entry_id)?);
            match params.identifier("Id", MAX_ENTRY_ID_CHARS) {
                Ok(Some(id)) => Ok(Entry { id, params }),
                _ => Err(invalid_entry_id()),
            }
        })
        .collect::<Result<Vec<_>>>()?;
    let mut seen = HashSet::new();
    if let Some(entry) = parsed.iter().find(|e| !seen.insert(e.id)) {
        return Err(Error::new(
            ErrorCode::BatchEntryIdsNotDistinct,
            format!("entry `Id` `{}` is used twice", entry.id),
        ));
    }
    Ok(parsed)
}

/// Builds the `Successful` and `Failed` result members from the outcome of
/// each entry, in entry order.
fn batch_result(outcomes: Vec<(&str, Result<Members>)>) -> Members {
    let mut successful = Vec::new();
    let mut failed = Vec::new();
    for (id, outcome) in outcomes {
        match outcome {
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

impl Broker {
    pub(super) fn send_message_batch(&mut self, params: Params<'_>) -> Result<Members> {
        self.run_batch(params, &["Body"], Self::send_to_queue)
    }

    pub(super) fn delete_message_batch(&mut self, params: Params<'_>) -> Result<Members> {
        self.run_batch(params, &["ReceiptHandle"], Self::delete_from_queue)
    }

    /// Checks the batch as a whole, then handles each entry with `single`
    /// after the required-parameter check of the single command, whose
    /// required parameters other than `QueueName` are `required`.
    fn run_batch(
        &mut self,
        params: Params<'_>,
        required: &[&str],
        single: fn(&mut Self, &str, Params<'_>) -> Result<Members>,
    ) -> Result<Members> {
        params.require(&["QueueName", "Entries"])?;
        let name = params.string("QueueName")?.expect("required");
        let entries = params.array("Entries")?.expect("required");
        self.queue_mut(name)?;
        let entries = parse_entries(entries)?;

        let outcomes = entries
            .into_iter()
            .map(|entry| {
                let outcome = entry
                    .params
                    .require(required)
                    .and_then(|()| single(self, name, entry.params));
                (entry.id, outcome)
            })
            .collect();
        Ok(batch_result(outcomes))
    }
}
