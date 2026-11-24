//! Queue tags (SPEC-feature section 3).

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
}

/// Tags to set, already validated.
pub struct TagUpdate(Vec<(TagKey, String)>);

impl TagUpdate {
    /// Validates the `Tags` object of `TagQueue`.
    pub fn parse(tags: &Map<String, Value>) -> Result<Self> {
        tags.iter()
            .map(|(key, value)| {
                let value = value
                    .as_str()
                    .filter(|v| v.len() <= MAX_VALUE_BYTES)
                    .ok_or_else(|| {
                        Error::new(
                            ErrorCode::InvalidParameterValue,
                            format!(
                                "tag value must be a string of at most {MAX_VALUE_BYTES} bytes"
                            ),
                        )
                    })?;
                Ok((TagKey::parse(key)?, value.to_owned()))
            })
            .collect::<Result<_>>()
            .map(Self)
    }
}

/// Keys to remove, already validated.
pub struct TagKeys(Vec<TagKey>);

impl TagKeys {
    /// Validates the `TagKeys` array of `UntagQueue`.
    pub fn parse(keys: &[Value]) -> Result<Self> {
        keys.iter()
            .map(|key| {
                let key = key.as_str().ok_or_else(|| {
                    Error::new(ErrorCode::InvalidParameterValue, "tag keys must be strings")
                })?;
                TagKey::parse(key)
            })
            .collect::<Result<_>>()
            .map(Self)
    }
}

/// The tags of one queue.
#[derive(Debug, Default)]
pub struct Tags(BTreeMap<TagKey, String>);

impl Tags {
    /// Sets the given tags, replacing the values of keys already present.
    /// Changes nothing if the queue would end up with more than 50 tags.
    pub fn set(&mut self, update: TagUpdate) -> Result<()> {
        let new_keys = update
            .0
            .iter()
            .filter(|(key, _)| !self.0.contains_key(key))
            .count();
        if self.0.len() + new_keys > MAX_TAGS {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                format!("a queue carries at most {MAX_TAGS} tags"),
            ));
        }
        self.0.extend(update.0);
        Ok(())
    }

    /// Removes the tags with these keys; keys that are not set are ignored.
    pub fn remove(&mut self, keys: TagKeys) {
        for key in keys.0 {
            self.0.remove(&key);
        }
    }

    pub fn to_json(&self) -> Value {
        Value::Object(
            self.0
                .iter()
                .map(|(key, value)| (key.0.clone(), Value::String(value.clone())))
                .collect(),
        )
    }
}
