//! Queue tags: validated keys and the per-queue tag set.

use std::collections::BTreeMap;

use serde_json::{Map, Value};

use crate::error::{Error, ErrorCode, Result};
use crate::naming;
use crate::request::Params;

const MAX_TAGS: usize = 50;
const MAX_KEY_CHARS: usize = 128;
const MAX_VALUE_BYTES: usize = 256;

/// A tag key that follows the key rule: 1 to 128 characters from
/// `A-Z a-z 0-9 _ -`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct TagKey(String);

impl TagKey {
    fn parse(text: &str) -> Result<Self> {
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
}

/// The tags a `TagQueue` request carries, validated but not yet applied.
pub struct TagUpdate(Vec<(TagKey, String)>);

impl TagUpdate {
    /// Validates the `Tags` object of a request (SPEC-feature section 3).
    pub fn parse(tags: Params<'_>) -> Result<Self> {
        tags.members()
            .map(|(key, value)| {
                let value = value
                    .as_str()
                    .filter(|v| v.len() <= MAX_VALUE_BYTES)
                    .ok_or_else(|| {
                        Error::new(
                            ErrorCode::InvalidParameterValue,
                            format!(
                                "tag `{key}` must be a string of at most {MAX_VALUE_BYTES} bytes"
                            ),
                        )
                    })?;
                Ok((TagKey::parse(key)?, value.to_owned()))
            })
            .collect::<Result<_>>()
            .map(Self)
    }
}

/// Keys named by an `UntagQueue` request.
pub struct TagKeys(Vec<TagKey>);

impl TagKeys {
    /// Validates the `TagKeys` array of a request.
    pub fn parse(keys: &[Value]) -> Result<Self> {
        keys.iter()
            .map(|key| {
                let text = key.as_str().ok_or_else(|| {
                    Error::new(
                        ErrorCode::InvalidParameterValue,
                        "`TagKeys` must hold only strings",
                    )
                })?;
                TagKey::parse(text)
            })
            .collect::<Result<_>>()
            .map(Self)
    }
}

/// The tags of one queue.
#[derive(Default)]
pub struct Tags(BTreeMap<TagKey, String>);

impl Tags {
    /// Sets every given tag, or changes nothing if the queue would end up
    /// with more than 50.
    pub fn set(&mut self, update: TagUpdate) -> Result<()> {
        let added = update
            .0
            .iter()
            .filter(|(key, _)| !self.0.contains_key(key))
            .count();
        if self.0.len() + added > MAX_TAGS {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                format!("a queue carries at most {MAX_TAGS} tags"),
            ));
        }
        self.0.extend(update.0);
        Ok(())
    }

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
                .collect::<Map<_, _>>(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn update(pairs: &[(&str, &str)]) -> TagUpdate {
        TagUpdate(
            pairs
                .iter()
                .map(|(k, v)| (TagKey((*k).to_owned()), (*v).to_owned()))
                .collect(),
        )
    }

    fn code(result: Result<TagKey>) -> std::result::Result<TagKey, ErrorCode> {
        result.map_err(|e| e.code)
    }

    #[test]
    fn key_rule_counts_characters_not_bytes() {
        let longest = "a".repeat(128);
        assert_eq!(code(TagKey::parse(&longest)), Ok(TagKey(longest)));
        for bad in ["a".repeat(129).as_str(), "", "a b", "\u{e9}"] {
            assert_eq!(
                code(TagKey::parse(bad)),
                Err(ErrorCode::InvalidParameterValue),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn replacing_a_key_does_not_count_against_the_limit() {
        let mut tags = Tags::default();
        let many: Vec<(String, &str)> = (0..50).map(|n| (format!("k{n}"), "v")).collect();
        let pairs: Vec<(&str, &str)> = many.iter().map(|(k, v)| (k.as_str(), *v)).collect();
        assert_eq!(tags.set(update(&pairs)), Ok(()));
        assert_eq!(tags.set(update(&[("k0", "new")])), Ok(()));
        let err = tags.set(update(&[("extra", "v")])).unwrap_err();
        assert_eq!(err.code, ErrorCode::LimitExceeded);
        assert_eq!(tags.to_json()["k0"], "new");
    }
}
