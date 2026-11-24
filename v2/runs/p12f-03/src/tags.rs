//! Queue tags: validated keys and values (SPEC-feature section 3).

use serde_json::Value;

use crate::error::{Error, ErrorCode, Result};
use crate::naming;
use crate::request::Params;

pub const MAX_TAGS: usize = 50;
const MAX_KEY_CHARS: usize = 128;
const MAX_VALUE_BYTES: usize = 256;

/// A tag key: 1 to 128 characters from `A-Z a-z 0-9 _ -`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct TagKey(String);

/// A tag value: at most 256 bytes of UTF-8.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TagValue(String);

fn invalid(message: String) -> Error {
    Error::new(ErrorCode::InvalidParameterValue, message)
}

impl TagKey {
    pub fn new(key: &str) -> Result<Self> {
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

impl TagValue {
    fn new(key: &str, value: &Value) -> Result<Self> {
        match value.as_str() {
            Some(text) if text.len() <= MAX_VALUE_BYTES => Ok(Self(text.to_owned())),
            _ => Err(invalid(format!(
                "value of tag `{key}` must be a string of at most {MAX_VALUE_BYTES} bytes"
            ))),
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Reads the required `Tags` object: every member is a key and its value.
pub fn parse_tags(params: Params<'_>) -> Result<Vec<(TagKey, TagValue)>> {
    let tags = params.required_object("Tags")?;
    tags.entries()
        .map(|(key, value)| Ok((TagKey::new(key)?, TagValue::new(key, value)?)))
        .collect()
}

/// Reads the required `TagKeys` array.
pub fn parse_tag_keys(params: Params<'_>) -> Result<Vec<TagKey>> {
    let keys = params.required_array("TagKeys")?;
    keys.iter()
        .map(|key| match key.as_str() {
            Some(key) => TagKey::new(key),
            None => Err(invalid("`TagKeys` must contain only strings".to_owned())),
        })
        .collect()
}
