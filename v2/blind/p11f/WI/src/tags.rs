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
                format!("a tag key must be 1 to {MAX_KEY_CHARS} characters from A-Z a-z 0-9 _ -"),
            ))
        }
    }
}

/// A tag value: at most 256 bytes when encoded as UTF-8.
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

/// Parses the `Tags` object of `TagQueue`.
pub fn parse_tag_object(object: &Map<String, Value>) -> Result<Vec<(TagKey, TagValue)>> {
    object
        .iter()
        .map(|(key, value)| {
            let text = value.as_str().ok_or_else(|| {
                Error::new(
                    ErrorCode::InvalidParameterValue,
                    format!("the value of tag `{key}` must be a string"),
                )
            })?;
            Ok((TagKey::parse(key)?, TagValue::parse(text)?))
        })
        .collect()
}

/// Parses the `TagKeys` array of `UntagQueue`.
pub fn parse_tag_keys(array: &[Value]) -> Result<Vec<TagKey>> {
    array
        .iter()
        .map(|value| {
            let text = value.as_str().ok_or_else(|| {
                Error::new(
                    ErrorCode::InvalidParameterValue,
                    "every element of `TagKeys` must be a string",
                )
            })?;
            TagKey::parse(text)
        })
        .collect()
}

/// The tags of one queue.
#[derive(Debug, Default)]
pub struct Tags(BTreeMap<TagKey, TagValue>);

impl Tags {
    /// Sets every given tag, or none if the queue would end up with more than
    /// 50. `new` has distinct keys because it comes from a JSON object.
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
                .map(|(k, v)| (k.0.clone(), Value::String(v.0.clone())))
                .collect(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pairs(n: usize) -> Vec<(TagKey, TagValue)> {
        (0..n)
            .map(|i| {
                (
                    TagKey::parse(&format!("k{i}")).unwrap(),
                    TagValue::parse("v").unwrap(),
                )
            })
            .collect()
    }

    #[test]
    fn limit_counts_only_new_keys_and_is_all_or_nothing() {
        let mut tags = Tags::default();
        tags.set(pairs(50)).unwrap();
        tags.set(pairs(50)).unwrap();
        let mut extra = pairs(1);
        extra.push((
            TagKey::parse("other").unwrap(),
            TagValue::parse("").unwrap(),
        ));
        assert_eq!(tags.set(extra).unwrap_err().code, ErrorCode::LimitExceeded);
        assert_eq!(tags.0.len(), 50);
    }

    #[test]
    fn value_limit_is_in_bytes() {
        assert!(TagValue::parse(&"a".repeat(256)).is_ok());
        assert!(TagValue::parse(&"é".repeat(128)).is_ok());
        assert!(TagValue::parse(&"é".repeat(129)).is_err());
    }
}
