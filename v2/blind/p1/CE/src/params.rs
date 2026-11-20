use std::ops::RangeInclusive;

use serde_json::{Map, Value};

use crate::error::{Error, ErrorCode};

/// Typed access to the members of a JSON object, following the protocol rule
/// that a `null` member counts as absent.
pub(crate) struct Params<'a>(&'a Map<String, Value>);

impl<'a> Params<'a> {
    pub fn new(members: &'a Map<String, Value>) -> Self {
        Self(members)
    }

    pub fn get(&self, name: &str) -> Option<&'a Value> {
        self.0.get(name).filter(|value| !value.is_null())
    }

    /// Fails with `MissingParameter` for the first of `names` that is absent.
    pub fn require(&self, names: &[&str]) -> Result<(), Error> {
        match names.iter().find(|name| self.get(name).is_none()) {
            Some(name) => Err(Error::new(
                ErrorCode::MissingParameter,
                format!("`{name}` is required"),
            )),
            None => Ok(()),
        }
    }

    pub fn string(&self, name: &str) -> Result<Option<&'a str>, Error> {
        self.get(name)
            .map(|value| value.as_str().ok_or_else(|| invalid(name, "a string")))
            .transpose()
    }

    pub fn boolean(&self, name: &str) -> Result<Option<bool>, Error> {
        self.get(name)
            .map(|value| value.as_bool().ok_or_else(|| invalid(name, "a boolean")))
            .transpose()
    }

    /// An integer written without fraction or exponent, within `range`.
    pub fn integer(&self, name: &str, range: RangeInclusive<u32>) -> Result<Option<u32>, Error> {
        self.get(name)
            .map(|value| {
                value
                    .as_i64()
                    .and_then(|n| u32::try_from(n).ok())
                    .filter(|n| range.contains(n))
                    .ok_or_else(|| {
                        invalid(
                            name,
                            &format!("an integer from {} to {}", range.start(), range.end()),
                        )
                    })
            })
            .transpose()
    }
}

pub(crate) fn invalid(name: &str, expected: &str) -> Error {
    Error::new(
        ErrorCode::InvalidParameterValue,
        format!("`{name}` must be {expected}"),
    )
}
