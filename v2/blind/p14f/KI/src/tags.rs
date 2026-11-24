//! Queue tags (feature spec section 3): validated keys and the per-queue set.

use std::collections::BTreeMap;

use serde_json::{Map, Value};

use crate::error::{Error, ErrorCode, Result};
use crate::naming;

const MAX_TAGS: usize = 50;
const MAX_KEY_CHARS: usize = 128;
const MAX_VALUE_BYTES: usize = 256;

fn invalid(message: String) -> Error {
    Error::new(ErrorCode::InvalidParameterValue, message)
}

/// A tag key that follows the key rule; it can only be built by [`TagKey::parse`].
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

/// Tags to set, validated but not yet applied.
pub struct TagUpdate(Vec<(TagKey, String)>);

impl TagUpdate {
    /// Validates the members of a `Tags` object.
    pub fn parse(members: &Map<String, Value>) -> Result<Self> {
        members
            .iter()
            .map(|(key, value)| {
                let key = TagKey::parse(key)?;
                let value = value
                    .as_str()
                    .filter(|v| v.len() <= MAX_VALUE_BYTES)
                    .ok_or_else(|| {
                        invalid(format!(
                            "value of tag `{}` must be a string of at most {MAX_VALUE_BYTES} bytes",
                            key.as_str()
                        ))
                    })?;
                Ok((key, value.to_owned()))
            })
            .collect::<Result<Vec<_>>>()
            .map(Self)
    }
}

/// The tags of one queue.
#[derive(Debug, Default)]
pub struct Tags(BTreeMap<TagKey, String>);

impl Tags {
    /// Sets every tag of `update`, or fails with `LimitExceeded` and changes nothing.
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
                .map(|(key, value)| (key.as_str().to_owned(), Value::String(value.clone())))
                .collect(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_follow_the_identifier_rule() {
        let long = "k".repeat(128);
        for good in ["env-1_A", long.as_str()] {
            assert_eq!(TagKey::parse(good), Ok(TagKey(good.to_owned())), "{good:?}");
        }
        for bad in ["", "a b", "é", &"k".repeat(129)] {
            assert_eq!(
                TagKey::parse(bad).map_err(|e| e.code),
                Err(ErrorCode::InvalidParameterValue),
                "{bad:?}"
            );
        }
    }
}
