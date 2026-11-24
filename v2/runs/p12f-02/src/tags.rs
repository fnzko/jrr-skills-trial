//! Queue tags: validated keys and values and the per-queue tag set.

use std::collections::BTreeMap;

use serde_json::{Map, Value};

use crate::error::{Error, ErrorCode, Result};
use crate::naming;

const MAX_TAGS: usize = 50;
const MAX_KEY_CHARS: usize = 128;
const MAX_VALUE_BYTES: usize = 256;

/// A tag key: 1 to 128 characters from `A-Z a-z 0-9 _ -`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct TagKey(String);

impl TagKey {
    pub fn parse(text: &str) -> Result<Self> {
        if naming::is_identifier(text, MAX_KEY_CHARS) {
            Ok(Self(text.to_owned()))
        } else {
            Err(Error::new(
                ErrorCode::InvalidParameterValue,
                format!("tag key must be 1 to {MAX_KEY_CHARS} characters from A-Z a-z 0-9 _ -"),
            ))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A tag value: a string of at most 256 UTF-8 bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TagValue(String);

impl TagValue {
    pub fn parse(value: &Value) -> Result<Self> {
        match value.as_str() {
            Some(text) if text.len() <= MAX_VALUE_BYTES => Ok(Self(text.to_owned())),
            _ => Err(Error::new(
                ErrorCode::InvalidParameterValue,
                format!("tag value must be a string of at most {MAX_VALUE_BYTES} bytes"),
            )),
        }
    }
}

/// The tags of one queue.
#[derive(Debug, Default)]
pub struct Tags(BTreeMap<TagKey, TagValue>);

impl Tags {
    /// Sets every given tag, or changes nothing if the queue would end up
    /// with more than 50 tags.
    pub fn set_all(&mut self, tags: Vec<(TagKey, TagValue)>) -> Result<()> {
        let added = tags
            .iter()
            .filter(|(key, _)| !self.0.contains_key(key))
            .count();
        if self.0.len() + added > MAX_TAGS {
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
        Value::Object(
            self.0
                .iter()
                .map(|(key, TagValue(value))| {
                    (key.as_str().to_owned(), Value::String(value.clone()))
                })
                .collect::<Map<_, _>>(),
        )
    }
}
