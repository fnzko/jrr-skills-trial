//! Queue tags: validated keys and values and the per-queue tag set.

use std::collections::{BTreeMap, BTreeSet};

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
                format!("a tag key must be 1 to {MAX_KEY_CHARS} characters from A-Z a-z 0-9 _ -"),
            ))
        }
    }

    /// Reads a key from a JSON value that must be a string.
    pub fn from_json(value: &Value) -> Result<Self> {
        let text = value.as_str().ok_or_else(|| {
            Error::new(
                ErrorCode::InvalidParameterValue,
                "a tag key must be a string",
            )
        })?;
        Self::parse(text)
    }
}

/// A tag value: a string of at most 256 bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TagValue(String);

impl TagValue {
    pub fn from_json(value: &Value) -> Result<Self> {
        let text = value.as_str().ok_or_else(|| {
            Error::new(
                ErrorCode::InvalidParameterValue,
                "a tag value must be a string",
            )
        })?;
        if text.len() > MAX_VALUE_BYTES {
            return Err(Error::new(
                ErrorCode::InvalidParameterValue,
                format!("a tag value must be at most {MAX_VALUE_BYTES} bytes"),
            ));
        }
        Ok(Self(text.to_owned()))
    }
}

/// The tags of one queue.
#[derive(Debug, Default)]
pub struct Tags(BTreeMap<TagKey, TagValue>);

impl Tags {
    /// Sets every given tag, replacing existing values. Fails with
    /// `LimitExceeded`, changing nothing, if the queue would carry more than
    /// 50 tags afterwards.
    pub fn set(&mut self, update: Vec<(TagKey, TagValue)>) -> Result<()> {
        let new_keys = update
            .iter()
            .map(|(key, _)| key)
            .filter(|key| !self.0.contains_key(key))
            .collect::<BTreeSet<_>>()
            .len();
        if self.0.len() + new_keys > MAX_TAGS {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                format!("a queue carries at most {MAX_TAGS} tags"),
            ));
        }
        self.0.extend(update);
        Ok(())
    }

    /// Removes the tags with these keys; absent keys are ignored.
    pub fn remove(&mut self, keys: &[TagKey]) {
        for key in keys {
            self.0.remove(key);
        }
    }

    /// The `Tags` member of a `ListQueueTags` result.
    pub fn to_json(&self) -> Value {
        Value::Object(
            self.0
                .iter()
                .map(|(key, value)| (key.0.clone(), Value::String(value.0.clone())))
                .collect::<Map<_, _>>(),
        )
    }
}
