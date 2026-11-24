//! Queue tags (SPEC-feature section 3).

use std::collections::BTreeMap;

use serde_json::{Value, json};

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

/// A tag value: at most 256 bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TagValue(String);

impl TagValue {
    pub fn parse(value: &Value) -> Result<Self> {
        match value.as_str() {
            Some(text) if text.len() <= MAX_VALUE_BYTES => Ok(Self(text.to_owned())),
            _ => Err(Error::new(
                ErrorCode::InvalidParameterValue,
                format!("a tag value must be a string of at most {MAX_VALUE_BYTES} bytes"),
            )),
        }
    }
}

/// The tags of one queue.
#[derive(Debug, Default)]
pub struct Tags(BTreeMap<TagKey, TagValue>);

impl Tags {
    /// Sets every given tag, replacing existing values; fails with
    /// `LimitExceeded` and changes nothing if more than 50 tags would result.
    pub fn set(&mut self, tags: Vec<(TagKey, TagValue)>) -> Result<()> {
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

    pub fn remove(&mut self, keys: &[TagKey]) {
        for key in keys {
            self.0.remove(key);
        }
    }

    pub fn to_json(&self) -> Value {
        let map: BTreeMap<&str, &str> = self
            .0
            .iter()
            .map(|(key, value)| (key.0.as_str(), value.0.as_str()))
            .collect();
        json!(map)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tag(key: &str) -> (TagKey, TagValue) {
        (
            TagKey::parse(key).unwrap(),
            TagValue::parse(&json!("v")).unwrap(),
        )
    }

    #[test]
    fn the_fifty_first_distinct_key_is_refused_but_replacing_is_not() {
        let mut tags = Tags::default();
        tags.set((0..50).map(|i| tag(&format!("k{i}"))).collect())
            .unwrap();
        let err = tags.set(vec![tag("extra")]).unwrap_err();
        assert_eq!(err.code, ErrorCode::LimitExceeded);
        tags.set(vec![tag("k0")]).unwrap();
    }

    #[test]
    fn keys_and_values_follow_their_rules() {
        let longest = "k".repeat(128);
        assert_eq!(TagKey::parse(&longest), Ok(TagKey(longest.clone())));
        for bad in ["", "a b", "é", &"k".repeat(129)] {
            assert_eq!(
                TagKey::parse(bad).unwrap_err().code,
                ErrorCode::InvalidParameterValue
            );
        }
        assert_eq!(TagValue::parse(&json!("")), Ok(TagValue(String::new())));
        assert_eq!(
            TagValue::parse(&json!("é".repeat(128))),
            Ok(TagValue("é".repeat(128)))
        );
        for bad in [json!("é".repeat(129)), json!(1), json!(null)] {
            assert_eq!(
                TagValue::parse(&bad).unwrap_err().code,
                ErrorCode::InvalidParameterValue,
                "{bad}"
            );
        }
    }
}
