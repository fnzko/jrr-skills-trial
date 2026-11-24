//! Queue tag keys and values (feature spec section 3).

use crate::error::{Error, ErrorCode, Result};
use crate::naming;

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

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Checks the size of a tag value.
pub fn check_value(key: &str, value: &str) -> Result<()> {
    if value.len() > MAX_VALUE_BYTES {
        return Err(Error::new(
            ErrorCode::InvalidParameterValue,
            format!("value of tag `{key}` is longer than {MAX_VALUE_BYTES} bytes"),
        ));
    }
    Ok(())
}
