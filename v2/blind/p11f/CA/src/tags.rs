//! Queue tags (SPEC-feature section 3).

use std::collections::BTreeMap;

use serde_json::Value;

use crate::error::{Error, ErrorCode, Result};
use crate::naming;

const MAX_TAGS: usize = 50;
const MAX_KEY_CHARS: usize = 128;
const MAX_VALUE_BYTES: usize = 256;

fn invalid_value(message: String) -> Error {
    Error::new(ErrorCode::InvalidParameterValue, message)
}

/// A tag key: 1 to 128 characters from `A-Z a-z 0-9 _ -`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct TagKey(String);

impl TagKey {
    pub fn parse(text: &str) -> Result<Self> {
        if naming::is_identifier(text, MAX_KEY_CHARS) {
            Ok(Self(text.to_owned()))
        } else {
            Err(invalid_value(format!(
                "tag key `{text}` must be 1 to {MAX_KEY_CHARS} characters from A-Z a-z 0-9 _ -"
            )))
        }
    }
}

/// A tag value: a string of at most 256 bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TagValue(String);

impl TagValue {
    pub fn parse(value: &Value) -> Result<Self> {
        match value.as_str() {
            Some(text) if text.len() <= MAX_VALUE_BYTES => Ok(Self(text.to_owned())),
            _ => Err(invalid_value(format!(
                "a tag value must be a string of at most {MAX_VALUE_BYTES} bytes"
            ))),
        }
    }
}

/// The tags of one queue.
#[derive(Debug, Default)]
pub struct Tags(BTreeMap<TagKey, TagValue>);

impl Tags {
    /// Sets every tag, or none if the queue would end up with too many.
    pub fn set_all(&mut self, tags: Vec<(TagKey, TagValue)>) -> Result<()> {
        let new_keys = tags.iter().filter(|(k, _)| !self.0.contains_key(k)).count();
        if self.0.len() + new_keys > MAX_TAGS {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                format!("a queue carries at most {MAX_TAGS} tags"),
            ));
        }
        self.0.extend(tags);
        Ok(())
    }

    pub fn remove_all(&mut self, keys: &[TagKey]) {
        for key in keys {
            self.0.remove(key);
        }
    }

    pub fn to_json(&self) -> Value {
        self.0
            .iter()
            .map(|(k, v)| (k.0.clone(), Value::String(v.0.clone())))
            .collect()
    }
}
