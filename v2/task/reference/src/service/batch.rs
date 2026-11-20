use std::collections::HashSet;

use serde_json::{Value, json};

use super::{Outcome, Send, Service, fields};
use crate::error::{Code, Failure};
use crate::params::{Params, is_token};

const MAX_ENTRIES: usize = 10;

struct Entry<'a> {
    id: &'a str,
    params: Params<'a>,
}

fn entries(items: &[Value]) -> Result<Vec<Entry<'_>>, Failure> {
    if items.is_empty() {
        return Err(Failure::new(Code::EmptyBatchRequest, "Entries is empty"));
    }
    if items.len() > MAX_ENTRIES {
        return Err(Failure::new(
            Code::TooManyEntriesInBatchRequest,
            format!("Entries has more than {MAX_ENTRIES} entries"),
        ));
    }
    let invalid_id = || {
        Failure::new(
            Code::InvalidBatchEntryId,
            "every entry must be an object with a valid Id",
        )
    };
    let entries = items
        .iter()
        .map(|item| {
            let params = Params::new(item.as_object()?);
            let id = params.get("Id")?.as_str().filter(|id| is_token(id, 80))?;
            Some(Entry { id, params })
        })
        .collect::<Option<Vec<_>>>()
        .ok_or_else(invalid_id)?;
    let mut seen = HashSet::new();
    if !entries.iter().all(|entry| seen.insert(entry.id)) {
        return Err(Failure::new(
            Code::BatchEntryIdsNotDistinct,
            "two entries have the same Id",
        ));
    }
    Ok(entries)
}

#[derive(Default)]
struct BatchResult {
    successful: Vec<Value>,
    failed: Vec<Value>,
}

impl BatchResult {
    fn record(&mut self, id: &str, outcome: Outcome) {
        match outcome {
            Ok(mut result) => {
                result.insert("Id".into(), id.into());
                self.successful.push(Value::Object(result));
            }
            Err(failure) => self
                .failed
                .push(json!({"Id": id, "error": failure.to_json()})),
        }
    }

    fn into_outcome(self) -> Outcome {
        Ok(fields([
            ("Successful", Value::Array(self.successful)),
            ("Failed", Value::Array(self.failed)),
        ]))
    }
}

impl Service {
    pub(super) fn send_message_batch(&mut self, params: &Params) -> Outcome {
        params.require(&["QueueName", "Entries"])?;
        let name = params.required_string("QueueName")?;
        let items = params.required_array("Entries")?;
        self.queue(name)?;
        let mut result = BatchResult::default();
        for entry in entries(items)? {
            let outcome = entry
                .params
                .require(&["Body"])
                .and_then(|()| Send::parse(&entry.params))
                .and_then(|send| self.send(name, &send));
            result.record(entry.id, outcome);
        }
        result.into_outcome()
    }

    pub(super) fn delete_message_batch(&mut self, params: &Params) -> Outcome {
        params.require(&["QueueName", "Entries"])?;
        let name = params.required_string("QueueName")?;
        let items = params.required_array("Entries")?;
        self.queue(name)?;
        let mut result = BatchResult::default();
        for entry in entries(items)? {
            let outcome = entry
                .params
                .required_string("ReceiptHandle")
                .and_then(|handle| self.delete_by_handle(name, handle))
                .map(|()| super::Fields::new());
            result.record(entry.id, outcome);
        }
        result.into_outcome()
    }
}
