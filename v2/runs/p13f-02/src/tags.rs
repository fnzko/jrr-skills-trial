//! Queue tags: validated keys and the per-queue tag set.

use std::collections::BTreeMap;

use serde_json::{Value, json};

use crate::error::{Error, ErrorCode, Result};
use crate::naming;
use crate::request::Params;

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
                format!(
                    "tag key `{text}` must be 1 to {MAX_KEY_CHARS} characters from A-Z a-z 0-9 _ -"
                ),
            ))
        }
    }

    /// Reads the keys of a `TagKeys` array element by element.
    pub fn parse_all(values: &[Value]) -> Result<Vec<Self>> {
        values
            .iter()
            .map(|value| {
                let text = value.as_str().ok_or_else(|| {
                    Error::new(
                        ErrorCode::InvalidParameterValue,
                        "`TagKeys` must contain only strings",
                    )
                })?;
                Self::parse(text)
            })
            .collect()
    }
}

/// The tags of one queue.
#[derive(Debug, Default)]
pub struct Tags(BTreeMap<TagKey, String>);

impl Tags {
    /// Validates a `Tags` object without applying it.
    pub fn parse(params: Params<'_>) -> Result<Vec<(TagKey, String)>> {
        params
            .members()
            .map(|(name, value)| {
                let key = TagKey::parse(name)?;
                let value = value
                    .as_str()
                    .filter(|v| v.len() <= MAX_VALUE_BYTES)
                    .ok_or_else(|| {
                        Error::new(
                            ErrorCode::InvalidParameterValue,
                            format!(
                                "tag `{name}` must be a string of at most {MAX_VALUE_BYTES} bytes"
                            ),
                        )
                    })?;
                Ok((key, value.to_owned()))
            })
            .collect()
    }

    /// Sets the given tags, or changes nothing if that would exceed the limit.
    pub fn set(&mut self, tags: Vec<(TagKey, String)>) -> Result<()> {
        let new_keys = tags
            .iter()
            .filter(|(key, _)| !self.0.contains_key(key))
            .count();
        if self.0.len() + new_keys > MAX_TAGS {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                format!("a queue carries at most {MAX_TAGS} tags"),
            ));
        }
        self.0.extend(tags);
        Ok(())
    }

    pub fn remove(&mut self, keys: &[TagKey]) {
        for key in keys {
            self.0.remove(key);
        }
    }

    pub fn to_json(&self) -> Value {
        Value::Object(
            self.0
                .iter()
                .map(|(k, v)| (k.0.clone(), json!(v)))
                .collect(),
        )
    }
}
