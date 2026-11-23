//! Typed access to the members of a command object.
//!
//! The specification checks parameters in two passes: first every required
//! parameter must be present, then every present value must have the right
//! type and range. Callers therefore call [`Params::require`] before the
//! typed getters.

use std::ops::RangeInclusive;

use serde_json::{Map, Value};

use crate::error::{Error, ErrorCode, Result};

/// The members of one command object. A member whose value is `null` counts
/// as absent.
#[derive(Clone, Copy)]
pub struct Params<'a>(&'a Map<String, Value>);

impl<'a> Params<'a> {
    pub fn new(members: &'a Map<String, Value>) -> Self {
        Self(members)
    }

    pub fn get(&self, name: &str) -> Option<&'a Value> {
        self.0.get(name).filter(|value| !value.is_null())
    }

    /// Fails with `MissingParameter` for the first name in `names` that is absent.
    pub fn require(&self, names: &[&str]) -> Result<()> {
        match names.iter().find(|name| self.get(name).is_none()) {
            Some(name) => Err(missing(name)),
            None => Ok(()),
        }
    }

    pub fn string(&self, name: &str) -> Result<Option<&'a str>> {
        self.get(name)
            .map(|value| {
                value
                    .as_str()
                    .ok_or_else(|| invalid(format!("{name} must be a string")))
            })
            .transpose()
    }

    pub fn required_string(&self, name: &str) -> Result<&'a str> {
        self.string(name)?.ok_or_else(|| missing(name))
    }

    pub fn integer(&self, name: &str, range: RangeInclusive<u32>) -> Result<Option<u32>> {
        self.get(name)
            .map(|value| bounded_integer(name, value, range))
            .transpose()
    }

    pub fn required_integer(&self, name: &str, range: RangeInclusive<u32>) -> Result<u32> {
        self.integer(name, range)?.ok_or_else(|| missing(name))
    }
}

/// Reads `value` as an integer in `range`, naming `name` in the error.
pub fn bounded_integer(name: &str, value: &Value, range: RangeInclusive<u32>) -> Result<u32> {
    let Some(number) = integer_value(value) else {
        return Err(invalid(format!("{name} must be an integer")));
    };
    u32::try_from(number)
        .ok()
        .filter(|number| range.contains(number))
        .ok_or_else(|| {
            invalid(format!(
                "{name} must be between {} and {}",
                range.start(),
                range.end()
            ))
        })
}

/// An integer is a JSON number written without fraction or exponent.
/// Values beyond the `i64` range saturate: they are out of every range that
/// callers accept, and are still integers.
fn integer_value(value: &Value) -> Option<i64> {
    let Value::Number(number) = value else {
        return None;
    };
    // With `arbitrary_precision` the number keeps the text it was written as.
    let text = number.to_string();
    let digits = text.strip_prefix('-').unwrap_or(&text);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(text.parse().unwrap_or(if text.starts_with('-') {
        i64::MIN
    } else {
        i64::MAX
    }))
}

pub fn missing(name: &str) -> Error {
    Error::new(ErrorCode::MissingParameter, format!("{name} is required"))
}

pub fn invalid(message: impl Into<String>) -> Error {
    Error::new(ErrorCode::InvalidParameterValue, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn members(json: &str) -> Map<String, Value> {
        match serde_json::from_str(json) {
            Ok(Value::Object(map)) => map,
            other => panic!("test input is not an object: {other:?}"),
        }
    }

    fn code<T: std::fmt::Debug>(result: Result<T>) -> ErrorCode {
        match result {
            Err(error) => error.code,
            Ok(value) => panic!("expected an error, got {value:?}"),
        }
    }

    #[test]
    fn integers_accept_only_plain_integer_text() {
        let map = members(
            r#"{"a":30,"b":-0,"c":30.0,"d":3e1,"e":"30","f":99999999999999999999,"g":-5,"h":true}"#,
        );
        let params = Params::new(&map);
        assert_eq!(params.integer("a", 0..=100).unwrap(), Some(30));
        assert_eq!(params.integer("b", 0..=100).unwrap(), Some(0));
        for name in ["c", "d", "e", "f", "g", "h"] {
            assert_eq!(
                code(params.integer(name, 0..=100)),
                ErrorCode::InvalidParameterValue,
                "{name}"
            );
        }
    }

    #[test]
    fn range_bounds_are_inclusive() {
        let map = members(r#"{"low":1,"high":10,"below":0,"above":11}"#);
        let params = Params::new(&map);
        assert_eq!(params.integer("low", 1..=10).unwrap(), Some(1));
        assert_eq!(params.integer("high", 1..=10).unwrap(), Some(10));
        assert_eq!(
            code(params.integer("below", 1..=10)),
            ErrorCode::InvalidParameterValue
        );
        assert_eq!(
            code(params.integer("above", 1..=10)),
            ErrorCode::InvalidParameterValue
        );
    }

    #[test]
    fn null_counts_as_absent() {
        let map = members(r#"{"a":null}"#);
        let params = Params::new(&map);
        assert_eq!(params.string("a").unwrap(), None);
        assert_eq!(code(params.require(&["a"])), ErrorCode::MissingParameter);
        assert_eq!(
            code(params.required_string("a")),
            ErrorCode::MissingParameter
        );
    }

    #[test]
    fn strings_reject_other_types() {
        let map = members(r#"{"a":1}"#);
        assert_eq!(
            code(Params::new(&map).string("a")),
            ErrorCode::InvalidParameterValue
        );
    }
}
