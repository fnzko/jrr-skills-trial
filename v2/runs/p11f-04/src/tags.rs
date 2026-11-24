//! Queue tags (feature spec section 3): validated keys and values.

use std::collections::BTreeMap;

use serde_json::{Map, Value};

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

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A tag value: a string of at most 256 bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TagValue(String);

impl TagValue {
    pub fn parse(value: &Value) -> Result<Self> {
        match value.as_str() {
            Some(text) if text.len() <= MAX_VALUE_BYTES => Ok(Self(text.to_owned())),
            _ => Err(invalid(format!(
                "a tag value must be a string of at most {MAX_VALUE_BYTES} bytes"
            ))),
        }
    }
}

/// The tags of one queue.
#[derive(Debug, Default)]
pub struct Tags(BTreeMap<TagKey, TagValue>);

impl Tags {
    /// Validates the members of a `Tags` object.
    pub fn parse_given(members: &Map<String, Value>) -> Result<BTreeMap<TagKey, TagValue>> {
        members
            .iter()
            .map(|(key, value)| Ok((TagKey::parse(key)?, TagValue::parse(value)?)))
            .collect()
    }

    /// Sets every given tag, replacing existing values. Fails with
    /// `LimitExceeded`, changing nothing, if more than 50 tags would result.
    pub fn set(&mut self, given: BTreeMap<TagKey, TagValue>) -> Result<()> {
        let added = given.keys().filter(|k| !self.0.contains_key(*k)).count();
        if self.0.len() + added > MAX_TAGS {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                format!("a queue carries at most {MAX_TAGS} tags"),
            ));
        }
        self.0.extend(given);
        Ok(())
    }

    /// Removes the tags with these keys; absent keys are ignored.
    pub fn remove(&mut self, keys: &[TagKey]) {
        for key in keys {
            self.0.remove(key);
        }
    }

    pub fn to_json(&self) -> Value {
        Value::Object(
            self.0
                .iter()
                .map(|(k, v)| (k.as_str().to_owned(), Value::String(v.0.clone())))
                .collect(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_follow_the_identifier_rule() {
        assert!(TagKey::parse("env-1_A").is_ok());
        assert!(TagKey::parse(&"k".repeat(128)).is_ok());
        for bad in ["", "a b", "é", &"k".repeat(129)] {
            assert_eq!(
                TagKey::parse(bad).unwrap_err().code,
                ErrorCode::InvalidParameterValue
            );
        }
    }

    #[test]
    fn values_are_limited_in_bytes() {
        assert!(TagValue::parse(&Value::from("")).is_ok());
        assert!(TagValue::parse(&Value::from("x".repeat(256))).is_ok());
        assert!(TagValue::parse(&Value::from("é".repeat(129))).is_err());
        assert!(TagValue::parse(&Value::from(1)).is_err());
    }
}
