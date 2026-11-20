//! Queue tags (SPEC-feature section 3).

use std::collections::BTreeMap;

use serde_json::{Map, Value};

use crate::error::{Error, ErrorCode, Result};
use crate::naming;
use crate::request::Params;

const MAX_TAGS: usize = 50;
const MAX_KEY_CHARS: usize = 128;
const MAX_VALUE_BYTES: usize = 256;

fn invalid_value(message: String) -> Error {
    Error::new(ErrorCode::InvalidParameterValue, message)
}

/// A tag key that follows the key rule.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct TagKey(String);

impl TagKey {
    pub fn parse(key: &str) -> Result<Self> {
        if naming::is_identifier(key, MAX_KEY_CHARS) {
            Ok(Self(key.to_owned()))
        } else {
            Err(invalid_value(format!(
                "tag key `{key}` must be 1 to {MAX_KEY_CHARS} characters from A-Z a-z 0-9 _ -"
            )))
        }
    }
}

/// A tag key with a value of at most 256 bytes.
#[derive(Debug)]
pub struct Tag {
    key: TagKey,
    value: String,
}

impl Tag {
    /// Reads the members of a `Tags` object (SPEC section 2, check 3).
    pub fn parse_all(tags: Params<'_>) -> Result<Vec<Self>> {
        tags.members()
            .map(|(key, value)| {
                let value = value.as_str().ok_or_else(|| {
                    invalid_value(format!("value of tag `{key}` must be a string"))
                })?;
                if value.len() > MAX_VALUE_BYTES {
                    return Err(invalid_value(format!(
                        "value of tag `{key}` must be at most {MAX_VALUE_BYTES} bytes"
                    )));
                }
                Ok(Self {
                    key: TagKey::parse(key)?,
                    value: value.to_owned(),
                })
            })
            .collect()
    }
}

/// The tags of one queue.
#[derive(Debug, Default)]
pub struct Tags(BTreeMap<TagKey, String>);

impl Tags {
    /// Sets every tag, or none if the queue would carry too many.
    pub fn set_all(&mut self, tags: Vec<Tag>) -> Result<()> {
        let added = tags.iter().filter(|t| !self.0.contains_key(&t.key)).count();
        if self.0.len() + added > MAX_TAGS {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                format!("a queue carries at most {MAX_TAGS} tags"),
            ));
        }
        self.0.extend(tags.into_iter().map(|t| (t.key, t.value)));
        Ok(())
    }

    pub fn remove_all(&mut self, keys: &[TagKey]) {
        for key in keys {
            self.0.remove(key);
        }
    }

    pub fn to_json(&self) -> Value {
        Value::Object(
            self.0
                .iter()
                .map(|(k, v)| (k.0.clone(), Value::String(v.clone())))
                .collect::<Map<_, _>>(),
        )
    }
}
