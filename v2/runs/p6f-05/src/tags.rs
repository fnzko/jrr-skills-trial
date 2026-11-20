//! Queue tags (SPEC-feature section 3).

use std::collections::BTreeMap;

use serde_json::{Value, json};

use crate::error::{Error, ErrorCode, Result};
use crate::naming;
use crate::request::Params;

const MAX_TAGS: usize = 50;
const MAX_KEY_CHARS: usize = 128;
const MAX_VALUE_BYTES: usize = 256;

fn invalid(message: String) -> Error {
    Error::new(ErrorCode::InvalidParameterValue, message)
}

/// Checks a tag key against the key rule.
pub fn check_key(key: &str) -> Result<()> {
    if naming::is_identifier(key, MAX_KEY_CHARS) {
        Ok(())
    } else {
        Err(invalid(format!(
            "tag key `{key}` must be 1 to {MAX_KEY_CHARS} characters from A-Z a-z 0-9 _ -"
        )))
    }
}

/// The tags of one queue, ordered by key.
#[derive(Debug, Default)]
pub struct Tags(BTreeMap<String, String>);

impl Tags {
    /// Reads and validates the key/value pairs of the object parameter `name`.
    pub fn parse<'a>(params: Params<'a>, name: &str) -> Result<Vec<(&'a str, &'a str)>> {
        let object = params.object(name)?.expect("required");
        object
            .members()
            .map(|(key, value)| {
                check_key(key)?;
                let value = value
                    .as_str()
                    .filter(|v| v.len() <= MAX_VALUE_BYTES)
                    .ok_or_else(|| {
                        invalid(format!(
                            "value of tag `{key}` must be a string of at most {MAX_VALUE_BYTES} bytes"
                        ))
                    })?;
                Ok((key, value))
            })
            .collect()
    }

    /// Sets every pair, or none and `LimitExceeded` if the queue would then
    /// carry more than 50 tags.
    pub fn set(&mut self, pairs: &[(&str, &str)]) -> Result<()> {
        let new_keys = pairs
            .iter()
            .filter(|(key, _)| !self.0.contains_key(*key))
            .count();
        if self.0.len() + new_keys > MAX_TAGS {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                format!("a queue carries at most {MAX_TAGS} tags"),
            ));
        }
        for (key, value) in pairs {
            self.0.insert((*key).to_owned(), (*value).to_owned());
        }
        Ok(())
    }

    pub fn remove(&mut self, key: &str) {
        self.0.remove(key);
    }

    pub fn to_json(&self) -> Value {
        json!(self.0)
    }
}
