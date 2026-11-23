//! Typed reads of single JSON parameter values. A value of the wrong type or
//! outside its range is `InvalidParameterValue`.

use serde_json::Value;

use crate::error::Error;
use crate::types::Bounded;

pub fn string<'a>(name: &str, value: &'a Value) -> Result<&'a str, Error> {
    value
        .as_str()
        .ok_or_else(|| Error::invalid_value(format!("{name} must be a string")))
}

pub fn boolean(name: &str, value: &Value) -> Result<bool, Error> {
    value
        .as_bool()
        .ok_or_else(|| Error::invalid_value(format!("{name} must be a boolean")))
}

/// Integers are numbers written without fraction or exponent.
pub fn integer<T: Bounded>(name: &str, value: &Value) -> Result<T, Error> {
    value.as_i64().and_then(T::new).ok_or_else(|| {
        Error::invalid_value(format!(
            "{name} must be an integer from {} to {}",
            T::MIN,
            T::MAX
        ))
    })
}
