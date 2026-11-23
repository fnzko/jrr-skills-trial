//! Queue tags: validated keys and values, and the per-queue tag set.

use std::collections::BTreeMap;

use serde_json::{Map, Value};

use crate::error::{Error, ErrorCode, Result};

pub const MAX_KEY_CHARS: usize = 128;
const MAX_VALUE_BYTES: usize = 256;
const MAX_TAGS: usize = 50;

/// The tags of one queue. Keys are unique and the set never exceeds
/// `MAX_TAGS` entries.
#[derive(Debug, Default)]
pub struct Tags {
    entries: BTreeMap<String, String>,
}

/// Checks the length rule of a tag value (SPEC-feature section 3).
pub fn check_value(key: &str, value: &Value) -> Result<()> {
    match value.as_str() {
        Some(text) if text.len() <= MAX_VALUE_BYTES => Ok(()),
        _ => Err(Error::new(
            ErrorCode::InvalidParameterValue,
            format!("the value of tag `{key}` must be a string of at most {MAX_VALUE_BYTES} bytes"),
        )),
    }
}

impl Tags {
    /// Sets every tag in `new`, replacing existing values. Fails without
    /// changing anything if the result would hold more than `MAX_TAGS` tags.
    /// Values must already have passed [`check_value`].
    pub fn set_all(&mut self, new: &Map<String, Value>) -> Result<()> {
        let added = new
            .keys()
            .filter(|key| !self.entries.contains_key(key.as_str()))
            .count();
        if self.entries.len() + added > MAX_TAGS {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                format!("a queue carries at most {MAX_TAGS} tags"),
            ));
        }
        for (key, value) in new {
            if let Some(text) = value.as_str() {
                self.entries.insert(key.clone(), text.to_owned());
            }
        }
        Ok(())
    }

    pub fn remove(&mut self, key: &str) {
        self.entries.remove(key);
    }

    pub fn to_json(&self) -> Value {
        Value::Object(
            self.entries
                .iter()
                .map(|(k, v)| (k.clone(), Value::String(v.clone())))
                .collect(),
        )
    }
}
