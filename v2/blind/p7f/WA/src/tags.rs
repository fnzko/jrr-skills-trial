//! Queue tags: validated keys and values and the per-queue tag set.

use std::collections::BTreeMap;

use crate::error::{Error, ErrorCode, Result};
use crate::naming;

pub const MAX_TAGS: usize = 50;
const MAX_KEY_CHARS: usize = 128;
const MAX_VALUE_BYTES: usize = 256;

fn invalid(message: String) -> Error {
    Error::new(ErrorCode::InvalidParameterValue, message)
}

/// A tag key: 1 to 128 characters from `A-Z a-z 0-9 _ -`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct TagKey(String);

impl TagKey {
    pub fn parse(key: &str) -> Result<Self> {
        if naming::is_identifier(key, MAX_KEY_CHARS) {
            Ok(Self(key.to_owned()))
        } else {
            Err(invalid(format!(
                "tag key `{key}` must be 1 to {MAX_KEY_CHARS} characters from A-Z a-z 0-9 _ -"
            )))
        }
    }
}

/// A tag value: at most 256 bytes of UTF-8.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TagValue(String);

impl TagValue {
    pub fn parse(value: &str) -> Result<Self> {
        if value.len() <= MAX_VALUE_BYTES {
            Ok(Self(value.to_owned()))
        } else {
            Err(invalid(format!(
                "a tag value must be at most {MAX_VALUE_BYTES} bytes"
            )))
        }
    }
}

/// The tags of one queue.
#[derive(Debug, Default)]
pub struct Tags(BTreeMap<TagKey, TagValue>);

impl Tags {
    /// How many tags the queue would carry after setting `keys`.
    pub fn count_after_set<'a>(&self, keys: impl Iterator<Item = &'a TagKey>) -> usize {
        self.0.len() + keys.filter(|key| !self.0.contains_key(*key)).count()
    }

    pub fn set(&mut self, key: TagKey, value: TagValue) {
        self.0.insert(key, value);
    }

    pub fn remove(&mut self, key: &TagKey) {
        self.0.remove(key);
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.0.iter().map(|(k, v)| (k.0.as_str(), v.0.as_str()))
    }
}
