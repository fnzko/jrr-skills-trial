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
                format!("a tag key must be 1 to {MAX_KEY_CHARS} characters from A-Z a-z 0-9 _ -"),
            ))
        }
    }
}

/// A tag value: at most 256 bytes of UTF-8.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TagValue(String);

impl TagValue {
    pub fn parse(text: &str) -> Result<Self> {
        if text.len() <= MAX_VALUE_BYTES {
            Ok(Self(text.to_owned()))
        } else {
            Err(Error::new(
                ErrorCode::InvalidParameterValue,
                format!("a tag value must be at most {MAX_VALUE_BYTES} bytes"),
            ))
        }
    }
}

/// The tags of one queue.
#[derive(Debug, Default)]
pub struct Tags {
    entries: BTreeMap<TagKey, TagValue>,
}

impl Tags {
    /// Sets every given tag, replacing the values of known keys. Nothing is
    /// set if the queue would end up with more than 50 tags.
    pub fn set_all(&mut self, tags: Vec<(TagKey, TagValue)>) -> Result<()> {
        let added = tags
            .iter()
            .filter(|(key, _)| !self.entries.contains_key(key))
            .count();
        if self.entries.len() + added > MAX_TAGS {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                format!("a queue carries at most {MAX_TAGS} tags"),
            ));
        }
        self.entries.extend(tags);
        Ok(())
    }

    /// Removes the tags with these keys; unknown keys are ignored.
    pub fn remove_all(&mut self, keys: &[TagKey]) {
        for key in keys {
            self.entries.remove(key);
        }
    }

    /// The `Tags` member of a `ListQueueTags` result.
    pub fn to_json(&self) -> Value {
        Value::Object(
            self.entries
                .iter()
                .map(|(key, value)| (key.0.clone(), Value::String(value.0.clone())))
                .collect::<Map<_, _>>(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_and_values_are_checked_at_their_bounds() {
        assert!(TagKey::parse(&"k".repeat(128)).is_ok());
        for bad in ["", "a b", "é", &"k".repeat(129)] {
            assert!(TagKey::parse(bad).is_err(), "{bad:?}");
        }
        assert!(TagValue::parse("").is_ok());
        assert!(TagValue::parse(&"v".repeat(256)).is_ok());
        assert!(TagValue::parse(&"v".repeat(257)).is_err());
        // 129 two-byte characters are 258 bytes.
        assert!(TagValue::parse(&"é".repeat(129)).is_err());
    }
}
