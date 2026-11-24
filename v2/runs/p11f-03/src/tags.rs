//! Queue tags: validated keys and the per-queue tag set (SPEC-feature section 3).

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
    pub fn parse(key: &str) -> Result<Self> {
        if naming::is_identifier(key, MAX_KEY_CHARS) {
            Ok(Self(key.to_owned()))
        } else {
            Err(Error::new(
                ErrorCode::InvalidParameterValue,
                format!(
                    "tag key `{key}` must be 1 to {MAX_KEY_CHARS} characters from A-Z a-z 0-9 _ -"
                ),
            ))
        }
    }
}

/// Parses the members of a `Tags` object into keys and values.
pub fn parse_tags(members: &Map<String, Value>) -> Result<BTreeMap<TagKey, String>> {
    members
        .iter()
        .map(|(key, value)| {
            let value = value
                .as_str()
                .filter(|v| v.len() <= MAX_VALUE_BYTES)
                .ok_or_else(|| {
                    Error::new(
                        ErrorCode::InvalidParameterValue,
                        format!("value of tag `{key}` must be a string of at most {MAX_VALUE_BYTES} bytes"),
                    )
                })?;
            Ok((TagKey::parse(key)?, value.to_owned()))
        })
        .collect()
}

/// The tags of one queue.
#[derive(Debug, Default)]
pub struct Tags(BTreeMap<TagKey, String>);

impl Tags {
    /// Sets every given tag, or none if the queue would end up with more
    /// than 50.
    pub fn set(&mut self, new: BTreeMap<TagKey, String>) -> Result<()> {
        let added = new.keys().filter(|key| !self.0.contains_key(key)).count();
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
                .map(|(key, value)| (key.0.clone(), Value::String(value.clone())))
                .collect(),
        )
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn key(s: &str) -> TagKey {
        TagKey(s.to_owned())
    }

    #[test]
    fn keys_follow_the_identifier_rule() {
        let longest = "k".repeat(128);
        assert_eq!(TagKey::parse(&longest), Ok(key(&longest)));
        for bad in ["", "a b", "\u{e9}", &"k".repeat(129)] {
            assert_eq!(
                TagKey::parse(bad).map_err(|e| e.code),
                Err(ErrorCode::InvalidParameterValue),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn replacing_values_does_not_count_against_the_limit() {
        let mut tags = Tags::default();
        let full: BTreeMap<_, _> = (0..50)
            .map(|i| (key(&format!("k{i}")), "v".to_owned()))
            .collect();
        assert_eq!(tags.set(full), Ok(()));
        assert_eq!(
            tags.set(BTreeMap::from([(key("k0"), "w".to_owned())])),
            Ok(())
        );
        let over = tags.set(BTreeMap::from([(key("extra"), "w".to_owned())]));
        assert_eq!(over.map_err(|e| e.code), Err(ErrorCode::LimitExceeded));
        let mut expected: Map<String, Value> =
            (1..50).map(|i| (format!("k{i}"), json!("v"))).collect();
        expected.insert("k0".into(), json!("w"));
        assert_eq!(tags.to_json(), Value::Object(expected));
    }
}
