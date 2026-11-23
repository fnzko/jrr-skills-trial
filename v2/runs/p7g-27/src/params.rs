//! Typed access to the members of a request object.

use serde_json::{Map, Value};

use crate::error::{Error, Result};

/// The members of a command object. A `null` member counts as absent.
pub struct Params<'a>(pub &'a Map<String, Value>);

impl<'a> Params<'a> {
    pub fn get(&self, name: &str) -> Option<&'a Value> {
        self.0.get(name).filter(|value| !value.is_null())
    }

    pub fn require(&self, name: &str) -> Result<&'a Value> {
        self.get(name).ok_or_else(|| Error::missing(name))
    }

    /// Applies `parse` to the member if it is present.
    pub fn optional<T>(
        &self,
        name: &str,
        parse: impl FnOnce(&'a Value) -> Result<T>,
    ) -> Result<Option<T>> {
        self.get(name).map(parse).transpose()
    }
}

pub fn as_string<'a>(name: &str, value: &'a Value) -> Result<&'a str> {
    value
        .as_str()
        .ok_or_else(|| Error::invalid_value(format!("{name} must be a string")))
}

pub fn as_bool(name: &str, value: &Value) -> Result<bool> {
    value
        .as_bool()
        .ok_or_else(|| Error::invalid_value(format!("{name} must be a boolean")))
}

/// An integer written without fraction or exponent, within `min..=max`.
pub fn as_int(name: &str, value: &Value, min: u32, max: u32) -> Result<u32> {
    value
        .as_i64()
        .and_then(|n| u32::try_from(n).ok())
        .filter(|n| (min..=max).contains(n))
        .ok_or_else(|| {
            Error::invalid_value(format!("{name} must be an integer from {min} to {max}"))
        })
}
