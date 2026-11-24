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

    /// Parses a JSON array element given as a tag key.
    pub fn from_json(value: &Value) -> Result<Self> {
        let text = value.as_str().ok_or_else(|| {
            Error::new(ErrorCode::InvalidParameterValue, "tag key must be a string")
        })?;
        Self::parse(text)
    }
}

/// A tag value: a string of at most 256 UTF-8 bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TagValue(String);

impl TagValue {
    pub fn from_json(value: &Value) -> Result<Self> {
        let text = value.as_str().ok_or_else(|| {
            Error::new(
                ErrorCode::InvalidParameterValue,
                "tag value must be a string",
            )
        })?;
        if text.len() > MAX_VALUE_BYTES {
            return Err(Error::new(
                ErrorCode::InvalidParameterValue,
                format!("tag value must be at most {MAX_VALUE_BYTES} bytes"),
            ));
        }
        Ok(Self(text.to_owned()))
    }
}

/// Validates the `Tags` object of `TagQueue`.
pub fn parse_tag_object(members: &Map<String, Value>) -> Result<Vec<(TagKey, TagValue)>> {
    members
        .iter()
        .map(|(key, value)| Ok((TagKey::parse(key)?, TagValue::from_json(value)?)))
        .collect()
}

/// The tags of one queue.
#[derive(Default)]
pub struct Tags(BTreeMap<TagKey, TagValue>);

impl Tags {
    /// Sets every given tag, or changes nothing if that would leave more
    /// than 50 tags.
    pub fn set(&mut self, new: Vec<(TagKey, TagValue)>) -> Result<()> {
        let added = new.iter().filter(|(k, _)| !self.0.contains_key(k)).count();
        if self.0.len() + added > MAX_TAGS {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                format!("a queue carries at most {MAX_TAGS} tags"),
            ));
        }
        self.0.extend(new);
        Ok(())
    }

    pub fn remove(&mut self, key: &TagKey) {
        self.0.remove(key);
    }

    pub fn to_json(&self) -> Value {
        Value::Object(
            self.0
                .iter()
                .map(|(k, v)| (k.0.clone(), Value::String(v.0.clone())))
                .collect(),
        )
    }
}
